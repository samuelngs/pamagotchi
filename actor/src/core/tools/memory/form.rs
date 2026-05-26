use super::super::context::{SessionContext, SessionState};
use super::helpers::{
    canonicalize_content_for_subjects, current_identity, current_profile, string_array,
};
use crate::store::{Memory, MemoryKind, MemorySource, MemorySubject};
use protocol::{IdentityId, MemoryId, ProfileId};
use serde_json::{Value, json};

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
        id: MemoryId(format!("mem-{}", super::super::util::uuid_v4())),
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
        created_at: super::super::util::now(),
        accessed_at: super::super::util::now(),
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
