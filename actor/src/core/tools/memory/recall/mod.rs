use super::super::context::{SessionContext, SessionState};
use super::helpers::{
    current_identity, current_profile, current_subject_relation, format_timestamp,
    global_recall_requested, relation_rank,
};
use crate::store::{MemoryKind, MemorySource, MemorySubjectType, RecallQuery};
use protocol::{IdentityId, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};
use std::time::Instant;

pub async fn recall(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(3) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let kind = args["kind"].as_str().and_then(MemoryKind::parse);
    let identity = args["identity"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| IdentityId(s.trim().to_string()));
    let profile = args["profile"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| ProfileId(s.trim().to_string()));
    let person = args["person"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| PersonId(s.trim().to_string()));

    let embedding = match ctx.router.embed(&[query]).await {
        Ok(vecs) if !vecs.is_empty() => vecs.into_iter().next(),
        _ => None,
    };
    let make_query = |limit: usize| match embedding.clone() {
        Some(embedding) => RecallQuery::by_embedding(embedding, limit),
        None => RecallQuery::by_text(query, limit),
    };
    let current = ctx.messages.first();

    let has_explicit_scope = identity.is_some() || profile.is_some() || person.is_some();
    let global_scope = global_recall_requested(args);
    let mut recall = make_query(limit).with_offset(offset);
    if let Some(kind) = kind.clone() {
        recall = recall.with_kind(kind);
    }
    if let Some(identity) = identity {
        recall = recall.with_identity(identity);
    }
    if let Some(profile) = profile {
        recall = recall.with_profile(profile);
    }
    if let Some(person) = person {
        recall = recall.with_person(person);
    }
    if !has_explicit_scope && !global_scope {
        if let Some(profile) = current_profile(ctx) {
            recall = recall.with_profile(profile);
        } else if let Some(identity) = current_identity(ctx) {
            recall = recall.with_identity(identity);
        }
    }
    if args["include_sensitive"].as_bool().unwrap_or(false) {
        recall = recall.include_sensitive();
    } else if let Some(max_sensitivity) = args["max_sensitivity"].as_f64() {
        recall = recall.with_max_sensitivity(max_sensitivity.clamp(0.0, 1.0) as f32);
    }
    if args["include_superseded"].as_bool().unwrap_or(false) {
        recall = recall.include_superseded();
    }

    let recall_start = Instant::now();
    let memories = ctx.store.recall(&recall).await;
    let recall_latency_ms = recall_start.elapsed().as_millis() as u64;

    match memories {
        Ok(memories) if memories.is_empty() => {
            ctx.metrics.record_recall(recall_latency_ms, 0);
            json!({"memories": []}).to_string()
        }
        Ok(memories) => {
            ctx.metrics.record_recall(recall_latency_ms, memories.len());
            let mut ranked_items = Vec::new();
            for (search_rank, m) in memories.into_iter().enumerate() {
                remember_recalled_memory(state, &m.id);
                let created_at = format_timestamp(m.created_at);
                let accessed_at = format_timestamp(m.accessed_at);
                let mut subjects = Vec::new();
                for subject in &m.subjects {
                    let mut entry = json!({
                        "type": subject.subject_type.as_str(),
                        "id": subject.subject_id,
                        "role": subject.role,
                        "confidence": subject.confidence,
                    });
                    if subject.subject_type == MemorySubjectType::Person {
                        if let Ok(Some(p)) = ctx
                            .store
                            .get_person(&PersonId(subject.subject_id.clone()))
                            .await
                        {
                            if let Some(name) = &p.name {
                                entry["name"] = json!(name);
                            }
                        }
                    }
                    subjects.push(entry);
                }
                let source = match &m.source {
                    MemorySource::Conversation {
                        conversation_id,
                        identity_id,
                        profile_id,
                        person_id,
                        message_id,
                    } => json!({
                        "kind": "conversation",
                        "conversation_id": conversation_id.0,
                        "identity_id": identity_id.as_ref().map(|id| id.0.clone()),
                        "profile_id": profile_id.as_ref().map(|id| id.0.clone()),
                        "person_id": person_id.as_ref().map(|id| id.0.clone()),
                        "message_id": message_id,
                    }),
                    MemorySource::Consolidation { from_memories } => json!({
                        "kind": "consolidation",
                        "from_memories": from_memories.iter().map(|id| id.0.clone()).collect::<Vec<_>>(),
                    }),
                    MemorySource::Reflection => json!({"kind": "reflection"}),
                    MemorySource::External => json!({"kind": "external"}),
                };
                let relation = current_subject_relation(&m.subjects, current, ctx).await;
                ranked_items.push((
                    relation_rank(relation),
                    search_rank,
                    m.created_at,
                    json!({
                        "id": m.id.0,
                        "kind": m.kind.as_str(),
                        "memory_type": m.memory_type.as_str(),
                        "content": m.content,
                        "created": created_at,
                        "created_at": created_at,
                        "accessed_at": accessed_at,
                        "access_count": m.access_count,
                        "importance": m.importance,
                        "confidence": m.confidence,
                        "sensitivity": m.sensitivity,
                        "sensitivity_category": m.sensitivity_category,
                        "emotional_valence": m.emotional_valence,
                        "tags": m.tags,
                        "privacy_category": m.privacy_category.as_str(),
                        "visibility_scope": m.visibility_scope.as_str(),
                        "truth_status": m.truth_status.as_str(),
                        "evidence_message_ids": m.evidence_message_ids,
                        "evidence_quote": m.evidence_quote,
                        "evidence": m.evidence,
                        "expires_at": m.expires_at,
                        "stability": m.stability.as_str(),
                        "supersedes": m.supersedes.as_ref().map(|id| id.0.clone()),
                        "superseded_by": m.superseded_by.as_ref().map(|id| id.0.clone()),
                        "contradiction_group": m.contradiction_group,
                        "last_confirmed_at": m.last_confirmed_at,
                        "next_review_at": m.next_review_at,
                        "dedupe_key": m.dedupe_key,
                        "subjects": subjects,
                        "source": source,
                        "current_subject_relation": relation,
                        "rank_reason": relation,
                    }),
                ));
            }
            ranked_items.sort_by(
                |(rank_a, search_a, created_a, _), (rank_b, search_b, created_b, _)| {
                    rank_a
                        .cmp(rank_b)
                        .then_with(|| search_a.cmp(search_b))
                        .then_with(|| created_b.cmp(created_a))
                },
            );
            let items = ranked_items
                .into_iter()
                .map(|(_, _, _, item)| item)
                .collect::<Vec<_>>();
            json!({"memories": items}).to_string()
        }
        Err(e) => {
            ctx.metrics.record_recall(recall_latency_ms, 0);
            json!({"error": format!("{e}")}).to_string()
        }
    }
}

fn remember_recalled_memory(state: &mut SessionState, id: &MemoryId) {
    if state
        .recalled_memory_ids
        .iter()
        .any(|existing| existing == id)
    {
        return;
    }
    state.recalled_memory_ids.push(id.clone());
    const MAX_RECALLED_MEMORY_IDS: usize = 32;
    if state.recalled_memory_ids.len() > MAX_RECALLED_MEMORY_IDS {
        let overflow = state.recalled_memory_ids.len() - MAX_RECALLED_MEMORY_IDS;
        state.recalled_memory_ids.drain(0..overflow);
    }
}

#[cfg(test)]
mod tests;
