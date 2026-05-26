use super::context::{SessionContext, SessionState};
use crate::store::{
    Memory, MemoryKind, MemorySource, MemorySubject, MemorySubjectType, MemoryUpdate, RecallQuery,
};
use inference::Tool;
use protocol::{IdentityId, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "recall_memories".into(),
            description: "Search memories by topic or keywords. Defaults to the current profile boundary; use scope=global only when intentionally searching across profiles.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default 3)",
                        "default": 3
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["episodic", "semantic", "procedural"],
                        "description": "Optional memory kind filter"
                    },
                    "identity": {
                        "type": "string",
                        "description": "Identity ID to restrict account-specific recall to."
                    },
                    "profile": {
                        "type": "string",
                        "description": "Profile ID to restrict recall to. Defaults to the current speaker profile when available."
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID to restrict recall to for verified person-level memories."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["current", "global"],
                        "description": "Recall scope. Defaults to current profile. Use global only when intentionally searching across profiles."
                    },
                    "global": {
                        "type": "boolean",
                        "description": "Deprecated alias for scope=global."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Skip this many results. Use to paginate: first call with offset 0, then offset 3 for more.",
                        "default": 0
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "form_memory".into(),
            description: "Save something worth remembering. User-specific facts are saved to the current profile by default; use promote_profile_memory_to_person only after verification.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "What to remember. Names are display labels, not identity keys."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["episodic", "semantic", "procedural"],
                        "description": "episodic = what happened, semantic = facts/knowledge, procedural = how to do things"
                    },
                    "importance": {
                        "type": "number",
                        "description": "0.0 to 1.0, how important this is",
                        "default": 0.5
                    },
                    "subject_profile_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Profile IDs this memory is about. Defaults to the current speaker profile."
                    },
                    "subject_identity_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Identity IDs this memory is about when the fact is account-specific."
                    },
                },
                "required": ["content", "kind"]
            }),
        },
        Tool {
            name: "promote_profile_memory_to_person".into(),
            description: "Deliberately promote a profile-level memory to a verified person grouping. Use only with explicit confirmation or strong verified evidence.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "Memory ID to promote"
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID that should become a subject of this memory"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Evidence or reason for promoting this memory"
                    }
                },
                "required": ["memory_id", "person"]
            }),
        },
        Tool {
            name: "demote_person_memory_to_profile".into(),
            description: "Move an over-broad person-level memory back to a profile subject without deleting the memory.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "Memory ID to demote"
                    },
                    "profile": {
                        "type": "string",
                        "description": "Profile ID that should own this memory"
                    },
                    "person": {
                        "type": "string",
                        "description": "Optional person ID to remove. If omitted, all person subjects are removed."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for demoting this memory"
                    }
                },
                "required": ["memory_id", "profile"]
            }),
        },
        Tool {
            name: "forget_memory".into(),
            description: "Remove a memory that's no longer relevant.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "ID of the memory to forget"
                    }
                },
                "required": ["memory_id"]
            }),
        },
    ]
}

fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

fn clean_display_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn canonicalize_content_for_subjects(
    content: &str,
    subjects: &[MemorySubject],
    ctx: &SessionContext,
) -> String {
    if subjects.is_empty() || content.starts_with("Subjects involved:\n") {
        return content.to_string();
    }

    let mut lines = vec!["Subjects involved:".to_string()];
    for subject in subjects {
        let label = if subject.subject_type == MemorySubjectType::Person {
            ctx.store
                .get_person(&PersonId(subject.subject_id.clone()))
                .await
                .ok()
                .flatten()
                .and_then(|person| person.name.map(|name| clean_display_name(&name)))
                .filter(|name| !name.is_empty())
        } else {
            None
        };
        if let Some(name) = label {
            lines.push(format!(
                "- {} {} (display name: {})",
                subject.subject_type.as_str(),
                subject.subject_id,
                name
            ));
        } else {
            lines.push(format!(
                "- {} {}",
                subject.subject_type.as_str(),
                subject.subject_id
            ));
        }
    }
    lines.push(format!("Memory: {content}"));
    lines.join("\n")
}

fn memory_body(content: &str) -> String {
    content
        .rsplit_once("\nMemory: ")
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|| content.to_string())
}

fn has_subject(subjects: &[MemorySubject], target: &MemorySubject) -> bool {
    subjects.iter().any(|subject| {
        subject.subject_type == target.subject_type && subject.subject_id == target.subject_id
    })
}

async fn current_subject_relation(
    subjects: &[MemorySubject],
    current: Option<&protocol::InboundMessage>,
    ctx: &SessionContext,
) -> &'static str {
    if let Some(current) = current {
        if let Some(identity) = &current.identity {
            if subjects.iter().any(|s| {
                s.subject_type == MemorySubjectType::Identity && s.subject_id == identity.0
            }) {
                return "same_identity";
            }
        }
        if let Some(profile) = &current.profile {
            if subjects
                .iter()
                .any(|s| s.subject_type == MemorySubjectType::Profile && s.subject_id == profile.0)
            {
                return "same_profile";
            }
        }
        if let Some(person) = &current.person {
            if subjects
                .iter()
                .any(|s| s.subject_type == MemorySubjectType::Person && s.subject_id == person.0)
            {
                return "same_person";
            }
            for subject in subjects
                .iter()
                .filter(|s| s.subject_type == MemorySubjectType::Profile)
            {
                if let Ok(Some((_subject_person, link))) = ctx
                    .store
                    .get_person_for_profile(&ProfileId(subject.subject_id.clone()))
                    .await
                {
                    if link.person_id == *person {
                        return match link.status.as_str() {
                            "verified" => "verified_same_person_profile",
                            "likely" => "likely_same_person_profile",
                            _ => "different_subject",
                        };
                    }
                }
            }
        }
        if !subjects.is_empty() {
            return "different_subject";
        }
        return "unlinked";
    }
    "unknown"
}

fn relation_rank(relation: &str) -> u8 {
    match relation {
        "same_profile" => 0,
        "same_identity" => 1,
        "same_person" => 2,
        "verified_same_person_profile" => 3,
        "likely_same_person_profile" => 4,
        "unlinked" => 5,
        "unknown" => 6,
        _ => 7,
    }
}

fn string_array(value: &Value) -> impl Iterator<Item = String> + '_ {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

fn current_identity(ctx: &SessionContext) -> Option<IdentityId> {
    ctx.messages.first().and_then(|m| m.identity.clone())
}

fn current_profile(ctx: &SessionContext) -> Option<ProfileId> {
    ctx.messages.first().and_then(|m| m.profile.clone())
}

fn global_recall_requested(args: &Value) -> bool {
    matches!(args["scope"].as_str(), Some("global")) || args["global"].as_bool().unwrap_or(false)
}

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

pub async fn form(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let raw_content = args["content"].as_str().unwrap_or("").to_string();
    let kind = args["kind"]
        .as_str()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Episodic);
    let importance = args["importance"].as_f64().unwrap_or(0.5) as f32;

    let explicit_profile_ids = string_array(&args["subject_profile_ids"]).collect::<Vec<_>>();
    let explicit_person_ids = string_array(&args["subject_person_ids"]).collect::<Vec<_>>();
    let explicit_identity_ids = string_array(&args["subject_identity_ids"]).collect::<Vec<_>>();

    if !explicit_person_ids.is_empty() {
        return json!({
            "error": "form_memory no longer writes directly to person subjects. Save to the current profile first, then use promote_profile_memory_to_person after verification."
        })
        .to_string();
    }

    let current_identity_id = current_identity(ctx);
    let current_profile_id = current_profile(ctx);

    if let Some(current) = &current_profile_id {
        if explicit_profile_ids.iter().any(|id| id != &current.0) {
            return json!({
                "error": "Refusing to save memory to a different profile from the current message."
            })
            .to_string();
        }
    }
    if let Some(current) = &current_identity_id {
        if explicit_identity_ids.iter().any(|id| id != &current.0) {
            return json!({
                "error": "Refusing to save memory to a different identity from the current message."
            })
            .to_string();
        }
    }

    let mut subjects: Vec<MemorySubject> = explicit_identity_ids
        .into_iter()
        .map(|id| MemorySubject::identity(IdentityId(id), Some("about".into()), 1.0))
        .collect();
    subjects.extend(
        explicit_profile_ids
            .into_iter()
            .map(|id| MemorySubject::profile(ProfileId(id), Some("about".into()), 1.0)),
    );
    if subjects.is_empty() {
        if let Some(profile) = current_profile_id {
            subjects.push(MemorySubject::profile(profile, Some("about".into()), 1.0));
        } else if let Some(identity) = current_identity_id {
            subjects.push(MemorySubject::identity(identity, Some("about".into()), 1.0));
        }
    }

    let content = canonicalize_content_for_subjects(&raw_content, &subjects, ctx).await;
    let source_conversation = ctx
        .conversation
        .clone()
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()));

    let embedding = match ctx.router.embed(&[&content]).await {
        Ok(vecs) => vecs.into_iter().next(),
        Err(_) => None,
    };

    let memory = Memory {
        id: MemoryId(format!("mem-{}", super::util::uuid_v4())),
        kind,
        content,
        source: source_conversation
            .as_ref()
            .and_then(|conv| {
                ctx.messages.first().map(|m| MemorySource::Conversation {
                    conversation_id: conv.clone(),
                    identity_id: m.identity.clone(),
                    profile_id: m.profile.clone(),
                    person_id: m.person.clone(),
                    message_id: Some(m.message_id.clone()),
                })
            })
            .unwrap_or(MemorySource::Reflection),
        importance,
        sensitivity: 0.0,
        emotional_valence: 0.0,
        created_at: super::util::now(),
        accessed_at: super::util::now(),
        access_count: 0,
        tags: vec![],
        subjects,
        embedding,
    };

    match ctx.store.store_memory(&memory).await {
        Ok(id) => {
            state.memories_formed.push(id.clone());
            format!("Memory saved: {}", id.0)
        }
        Err(e) => format!("Failed to save memory: {e}"),
    }
}

pub async fn promote_profile_memory_to_person(args: &Value, ctx: &SessionContext) -> String {
    let Some(memory_id) = args["memory_id"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide memory_id."}).to_string();
    };
    let Some(person) = args["person"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide person."}).to_string();
    };

    let memory_id = MemoryId(memory_id.trim().to_string());
    let person = PersonId(person.trim().to_string());
    let memory = match ctx.store.get_memory(&memory_id).await {
        Ok(Some(memory)) => memory,
        Ok(None) => return json!({"error": "Memory not found."}).to_string(),
        Err(e) => return json!({"error": format!("{e}")}).to_string(),
    };

    if ctx.store.get_person(&person).await.ok().flatten().is_none() {
        return json!({"error": "Person not found."}).to_string();
    }

    let mut subjects = memory.subjects.clone();
    let promoted = MemorySubject::person(person.clone(), Some("about".into()), 1.0);
    if !has_subject(&subjects, &promoted) {
        subjects.push(promoted);
    }

    let content =
        canonicalize_content_for_subjects(&memory_body(&memory.content), &subjects, ctx).await;
    let update = MemoryUpdate {
        content: Some(content),
        importance: None,
        sensitivity: None,
        emotional_valence: None,
        tags: None,
        subjects: Some(subjects),
        embedding: None,
    };

    match ctx.store.update_memory(&memory_id, &update).await {
        Ok(()) => json!({
            "status": "promoted",
            "memory_id": memory_id.0,
            "person": person.0,
            "reason": args["reason"].as_str()
        })
        .to_string(),
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}

pub async fn demote_person_memory_to_profile(args: &Value, ctx: &SessionContext) -> String {
    let Some(memory_id) = args["memory_id"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide memory_id."}).to_string();
    };
    let Some(profile) = args["profile"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide profile."}).to_string();
    };

    let memory_id = MemoryId(memory_id.trim().to_string());
    let profile = ProfileId(profile.trim().to_string());
    let memory = match ctx.store.get_memory(&memory_id).await {
        Ok(Some(memory)) => memory,
        Ok(None) => return json!({"error": "Memory not found."}).to_string(),
        Err(e) => return json!({"error": format!("{e}")}).to_string(),
    };

    if ctx
        .store
        .get_profile(&profile)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return json!({"error": "Profile not found."}).to_string();
    }

    let remove_person = args["person"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    let mut subjects = memory
        .subjects
        .iter()
        .filter(|subject| {
            if subject.subject_type != MemorySubjectType::Person {
                return true;
            }
            match &remove_person {
                Some(person) => subject.subject_id != *person,
                None => false,
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    let demoted = MemorySubject::profile(profile.clone(), Some("about".into()), 1.0);
    if !has_subject(&subjects, &demoted) {
        subjects.push(demoted);
    }

    let content =
        canonicalize_content_for_subjects(&memory_body(&memory.content), &subjects, ctx).await;
    let update = MemoryUpdate {
        content: Some(content),
        importance: None,
        sensitivity: None,
        emotional_valence: None,
        tags: None,
        subjects: Some(subjects),
        embedding: None,
    };

    match ctx.store.update_memory(&memory_id, &update).await {
        Ok(()) => json!({
            "status": "demoted",
            "memory_id": memory_id.0,
            "profile": profile.0,
            "removed_person": remove_person,
            "reason": args["reason"].as_str()
        })
        .to_string(),
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}

pub async fn forget(args: &Value, ctx: &SessionContext) -> String {
    let id = args["memory_id"].as_str().unwrap_or("");
    match ctx.store.forget(&MemoryId(id.to_string())).await {
        Ok(true) => "Memory forgotten.".into(),
        Ok(false) => "Memory not found.".into(),
        Err(e) => format!("Error: {e}"),
    }
}
