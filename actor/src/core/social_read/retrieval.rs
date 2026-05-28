use crate::core::prompt::context::{OpenLoopCtx, RelevantMemoryCtx, SocialRelationCtx};
use crate::identity::{RelationStatus, SocialRelation};
use crate::store::{IntentRecord, Memory, MemoryType, RecallQuery, Store};
use protocol::{ConversationId, IdentityId, InboundMessage, PersonId, ProfileId};
use std::{collections::HashSet, sync::Arc};

pub(crate) async fn fetch_relevant_memories(
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    supplemental_query_text: &[&str],
    identity: Option<&IdentityId>,
    profile: Option<&ProfileId>,
    person: Option<&PersonId>,
) -> Vec<RelevantMemoryCtx> {
    let Some(query_text) = memory_pack_query(messages, supplemental_query_text) else {
        return vec![];
    };

    let mut seen = HashSet::new();
    let mut memories = Vec::new();

    if let Some(profile) = profile {
        append_recalled_memories(
            store,
            RecallQuery::by_text(&query_text, 4).with_profile(profile.clone()),
            "current_profile",
            &mut seen,
            &mut memories,
        )
        .await;
    }
    if let Some(identity) = identity {
        append_recalled_memories(
            store,
            RecallQuery::by_text(&query_text, 2).with_identity(identity.clone()),
            "current_identity",
            &mut seen,
            &mut memories,
        )
        .await;
    }
    if let Some(person) = person {
        append_recalled_memories(
            store,
            RecallQuery::by_text(&query_text, 2).with_person(person.clone()),
            "current_person",
            &mut seen,
            &mut memories,
        )
        .await;
    }

    memories.truncate(6);
    memories
}

fn memory_pack_query(
    messages: &[InboundMessage],
    supplemental_query_text: &[&str],
) -> Option<String> {
    let message_text = messages
        .iter()
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>();
    let supplemental_text = supplemental_query_text
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty());
    let query = message_text
        .into_iter()
        .chain(supplemental_text)
        .collect::<Vec<_>>()
        .join("\n");
    if query.is_empty() {
        None
    } else {
        Some(query.chars().take(512).collect())
    }
}

async fn append_recalled_memories(
    store: &Arc<dyn Store>,
    query: RecallQuery,
    scope: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<RelevantMemoryCtx>,
) {
    let Ok(memories) = store.recall(&query).await else {
        return;
    };
    for memory in memories {
        if seen.insert(memory.id.0.clone()) {
            out.push(relevant_memory_ctx(memory, scope));
        }
    }
}

fn relevant_memory_ctx(memory: Memory, scope: &str) -> RelevantMemoryCtx {
    RelevantMemoryCtx {
        id: memory.id.0,
        scope: scope.to_string(),
        memory_type: memory.memory_type.as_str().to_string(),
        truth_status: memory.truth_status.as_str().to_string(),
        importance: pct(memory.importance),
        confidence: pct(memory.confidence),
        content: memory.content,
    }
}

pub(crate) async fn fetch_relationship_memories(
    store: &Arc<dyn Store>,
    profile: Option<&ProfileId>,
    person: Option<&PersonId>,
) -> Vec<RelevantMemoryCtx> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    if let Some(profile) = profile {
        append_relationship_memories(
            store,
            RecallQuery::by_text("", 40)
                .with_profile(profile.clone())
                .with_memory_types(relationship_pack_memory_types()),
            "current_profile",
            &mut seen,
            &mut out,
        )
        .await;
    }
    if let Some(person) = person {
        append_relationship_memories(
            store,
            RecallQuery::by_text("", 40)
                .with_person(person.clone())
                .with_memory_types(relationship_pack_memory_types()),
            "current_person",
            &mut seen,
            &mut out,
        )
        .await;
    }
    out.sort_by(|a, b| {
        b.importance
            .cmp(&a.importance)
            .then_with(|| b.confidence.cmp(&a.confidence))
    });
    out.truncate(6);
    out
}

async fn append_relationship_memories(
    store: &Arc<dyn Store>,
    query: RecallQuery,
    scope: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<RelevantMemoryCtx>,
) {
    let Ok(memories) = store.recall(&query).await else {
        return;
    };
    for memory in memories {
        if !relationship_pack_memory_type(&memory.memory_type) {
            continue;
        }
        if seen.insert(memory.id.0.clone()) {
            out.push(relevant_memory_ctx(memory, scope));
        }
    }
}

fn relationship_pack_memory_type(memory_type: &MemoryType) -> bool {
    matches!(
        memory_type,
        MemoryType::Preference
            | MemoryType::StylePattern
            | MemoryType::Boundary
            | MemoryType::Commitment
            | MemoryType::OpenLoop
            | MemoryType::RelationshipFact
    )
}

fn relationship_pack_memory_types() -> Vec<MemoryType> {
    vec![
        MemoryType::Preference,
        MemoryType::StylePattern,
        MemoryType::Boundary,
        MemoryType::Commitment,
        MemoryType::OpenLoop,
        MemoryType::RelationshipFact,
    ]
}

pub(crate) async fn fetch_social_relations(
    store: &Arc<dyn Store>,
    person: Option<&PersonId>,
) -> Vec<SocialRelationCtx> {
    let Some(person) = person else {
        return vec![];
    };
    let Ok(mut relations) = store.get_relations(person).await else {
        return vec![];
    };
    relations.retain(|relation| {
        !matches!(
            relation.status,
            RelationStatus::Denied | RelationStatus::Outdated
        ) && relation.confidence >= 0.4
    });
    relations.sort_by(|a, b| social_relation_rank(b).cmp(&social_relation_rank(a)));

    let mut out = Vec::new();
    for relation in relations.into_iter().take(5) {
        out.push(social_relation_ctx(store, relation).await);
    }
    out
}

async fn social_relation_ctx(
    store: &Arc<dyn Store>,
    relation: SocialRelation,
) -> SocialRelationCtx {
    let person_a_name = resolve_person_name(store, &relation.person_a).await;
    let person_b_name = resolve_person_name(store, &relation.person_b).await;
    let asserted_by_name = if let Some(person) = relation.asserted_by.as_ref() {
        resolve_person_name(store, person).await
    } else {
        None
    };
    SocialRelationCtx {
        person_a: relation.person_a.0,
        person_a_name,
        person_b: relation.person_b.0,
        person_b_name,
        relation: relation.relation.as_str().to_string(),
        direction: relation.direction.as_str().to_string(),
        confidence: pct(relation.confidence),
        status: relation.status.as_str().to_string(),
        source_kind: relation.source_kind.as_str().to_string(),
        asserted_by: relation.asserted_by.map(|person| person.0),
        asserted_by_name,
        evidence: relation_evidence_summary(relation.evidence.as_ref()),
    }
}

async fn resolve_person_name(store: &Arc<dyn Store>, person_id: &PersonId) -> Option<String> {
    store
        .get_person(person_id)
        .await
        .ok()
        .flatten()
        .and_then(|person| person.name)
}

fn relation_evidence_summary(evidence: Option<&serde_json::Value>) -> Option<String> {
    let evidence = evidence?;
    if let Some(quote) = evidence
        .get("quote")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|quote| !quote.is_empty())
    {
        return Some(format!("quote: {}", truncate_prompt_value(quote, 120)));
    }

    if let Some(message_ids) = evidence
        .get("message_ids")
        .and_then(serde_json::Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .take(3)
                .collect::<Vec<_>>()
        })
        .filter(|ids| !ids.is_empty())
    {
        return Some(format!("messages {}", message_ids.join(", ")));
    }

    if let Some(message_id) = evidence
        .get("message_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return Some(format!("message {message_id}"));
    }

    let reason = evidence
        .get("reason")
        .or_else(|| {
            evidence
                .get("evidence")
                .and_then(|inner| inner.get("reason"))
        })
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty())?;
    Some(format!("reason: {}", truncate_prompt_value(reason, 120)))
}

fn truncate_prompt_value(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn social_relation_rank(relation: &SocialRelation) -> (u8, i32, i64) {
    let status_rank = match relation.status {
        RelationStatus::Confirmed => 3,
        RelationStatus::Stated => 2,
        RelationStatus::Hypothesis => 1,
        RelationStatus::Denied | RelationStatus::Outdated => 0,
    };
    (status_rank, pct(relation.confidence), relation.updated_at)
}

pub(crate) async fn fetch_open_loops(
    store: &Arc<dyn Store>,
    person: Option<&PersonId>,
    profile: Option<&ProfileId>,
    conversation: Option<&ConversationId>,
    now: i64,
) -> Vec<OpenLoopCtx> {
    match store
        .active_intents_for_context(person, profile, conversation, now, 5)
        .await
    {
        Ok(intents) => intents
            .into_iter()
            .map(|intent| open_loop_ctx(intent, now))
            .collect(),
        Err(_) => vec![],
    }
}

fn open_loop_ctx(intent: IntentRecord, now: i64) -> OpenLoopCtx {
    OpenLoopCtx {
        id: intent.id,
        kind: intent.kind,
        task: intent.task,
        due: intent
            .fire_at
            .map(|fire_at| describe_due_time(fire_at, now)),
        condition: intent.condition,
        source_memory: intent.source_memory.map(|id| id.0),
        priority: intent.priority,
    }
}

fn describe_due_time(fire_at: i64, now: i64) -> String {
    if fire_at <= now {
        let secs = (now - fire_at).max(0);
        if secs < 60 {
            "due now".into()
        } else {
            format!("overdue by {}", duration_words(secs))
        }
    } else {
        format!("in {}", duration_words(fire_at - now))
    }
}

fn duration_words(secs: i64) -> String {
    if secs < 60 {
        "less than 1 minute".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute".into()
        } else {
            format!("{m} minutes")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour".into()
        } else {
            format!("{h} hours")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day".into()
        } else {
            format!("{d} days")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week".into()
        } else {
            format!("{w} weeks")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month".into()
        } else {
            format!("{mo} months")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year".into()
        } else {
            format!("{y} years")
        }
    }
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}
