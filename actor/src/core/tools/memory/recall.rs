use super::super::context::SessionContext;
use super::helpers::{
    current_identity, current_profile, current_subject_relation, format_timestamp,
    global_recall_requested, relation_rank,
};
use crate::store::{MemoryKind, MemorySource, MemorySubjectType, RecallQuery};
use protocol::{IdentityId, PersonId, ProfileId};
use serde_json::{Value, json};

pub async fn recall(args: &Value, ctx: &SessionContext) -> String {
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

    let memories = ctx.store.recall(&recall).await;

    match memories {
        Ok(memories) if memories.is_empty() => json!({"memories": []}).to_string(),
        Ok(memories) => {
            let mut ranked_items = Vec::new();
            for m in memories {
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
                    m.created_at,
                    json!({
                        "id": m.id.0,
                        "kind": m.kind.as_str(),
                        "content": m.content,
                        "created": created_at,
                        "created_at": created_at,
                        "accessed_at": accessed_at,
                        "importance": m.importance,
                        "subjects": subjects,
                        "source": source,
                        "current_subject_relation": relation,
                        "rank_reason": relation,
                    }),
                ));
            }
            ranked_items.sort_by(|(rank_a, created_a, _), (rank_b, created_b, _)| {
                rank_a.cmp(rank_b).then_with(|| created_b.cmp(created_a))
            });
            let items = ranked_items
                .into_iter()
                .map(|(_, _, item)| item)
                .collect::<Vec<_>>();
            json!({"memories": items}).to_string()
        }
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}
