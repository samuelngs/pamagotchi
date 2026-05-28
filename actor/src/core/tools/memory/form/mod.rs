use super::super::context::{SessionContext, SessionState};
use super::helpers::{canonicalize_content_for_subjects, string_array};
use crate::store::{
    Memory, MemoryKind, MemorySource, MemoryStability, MemorySubject, MemoryType, PrivacyCategory,
    TruthStatus, VisibilityScope, memory_privacy_policy_for_subject, memory_stability_policy,
    memory_truth_status_policy, sensitive_memory_next_review_at,
};
use protocol::{IdentityId, InboundMessage, MemoryId, ProfileId};
use serde_json::{Value, json};

pub async fn form(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let raw_content = args["content"].as_str().unwrap_or("").to_string();
    let kind = args["kind"]
        .as_str()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Episodic);
    let importance = clamp_unit(args["importance"].as_f64().unwrap_or(0.5) as f32);
    let sensitivity = clamp_unit(args["sensitivity"].as_f64().unwrap_or(0.0) as f32);
    let emotional_valence = clamp_valence(args["emotional_valence"].as_f64().unwrap_or(0.0) as f32);
    let tags = string_array(&args["tags"]).collect::<Vec<_>>();
    let confidence = clamp_unit(args["confidence"].as_f64().unwrap_or(1.0) as f32);
    let memory_type = args["memory_type"]
        .as_str()
        .and_then(MemoryType::parse)
        .unwrap_or_default();
    let explicit_truth_status = args["truth_status"].as_str().and_then(TruthStatus::parse);
    let truth_status = memory_truth_status_policy(&memory_type, explicit_truth_status);
    let evidence_source_messages = evidence_source_messages(ctx, state);
    let explicit_evidence_message_ids =
        string_array(&args["evidence_message_ids"]).collect::<Vec<_>>();
    let evidence_messages =
        match selected_evidence_messages(&evidence_source_messages, &explicit_evidence_message_ids)
        {
            Ok(messages) => messages,
            Err(missing) => {
                return json!({
                    "error": format!(
                        "Evidence message ids are not available in the current action: {}",
                        missing.join(", ")
                    ),
                })
                .to_string();
            }
        };
    let mut evidence_message_ids = explicit_evidence_message_ids;
    if evidence_message_ids.is_empty() {
        evidence_message_ids = evidence_messages
            .iter()
            .map(|message| message.message_id.clone())
            .filter(|id| !id.is_empty())
            .collect();
    }
    let evidence_quote = args["evidence_quote"].as_str().map(str::to_string);
    let evidence = super::super::util::evidence_with_source_spans(
        args,
        serde_json::Value::Object(Default::default()),
    );
    let expires_at = args["expires_at"].as_i64();
    let explicit_stability = args["stability"].as_str().and_then(MemoryStability::parse);
    let stability = memory_stability_policy(&memory_type, &truth_status, explicit_stability);
    let subject_actor = args["subject_actor"].as_bool().unwrap_or(false);
    let sensitivity_category = args["sensitivity_category"].as_str().map(str::to_string);
    let explicit_privacy_category = args["privacy_category"]
        .as_str()
        .and_then(PrivacyCategory::parse);
    let explicit_visibility_scope = args["visibility_scope"]
        .as_str()
        .and_then(VisibilityScope::parse);
    let (privacy_category, visibility_scope) = memory_privacy_policy_for_subject(
        sensitivity,
        sensitivity_category.as_deref(),
        explicit_privacy_category,
        explicit_visibility_scope,
        subject_actor,
    );
    let dedupe_key = args["dedupe_key"].as_str().map(str::to_string);
    let supersedes = args["supersedes"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| MemoryId(id.to_string()));
    let contradiction_group = args["contradiction_group"]
        .as_str()
        .filter(|group| !group.trim().is_empty())
        .map(str::to_string);
    let last_confirmed_at = args["last_confirmed_at"].as_i64();
    let explicit_next_review_at = args["next_review_at"].as_i64();

    let explicit_profile_ids = string_array(&args["subject_profile_ids"]).collect::<Vec<_>>();
    let explicit_person_ids = string_array(&args["subject_person_ids"]).collect::<Vec<_>>();
    let explicit_identity_ids = string_array(&args["subject_identity_ids"]).collect::<Vec<_>>();

    if !explicit_person_ids.is_empty() {
        return json!({
            "error": "form_memory no longer writes directly to person subjects. Save to the current profile first, then use promote_profile_memory_to_person after verification."
        })
        .to_string();
    }
    if subject_actor
        && (!explicit_profile_ids.is_empty()
            || !explicit_identity_ids.is_empty()
            || !explicit_person_ids.is_empty())
    {
        return json!({
            "error": "Actor self memories cannot be mixed with profile, identity, or person subjects."
        })
        .to_string();
    }

    let source_message = source_message_for_evidence(&evidence_messages, &evidence_message_ids);

    let allowed_profile_ids = evidence_messages
        .iter()
        .filter_map(|message| message.profile.as_ref().map(|id| id.0.as_str()))
        .collect::<Vec<_>>();
    if !allowed_profile_ids.is_empty() {
        if explicit_profile_ids
            .iter()
            .any(|id| !allowed_profile_ids.contains(&id.as_str()))
        {
            return json!({
                "error": "Refusing to save memory to a profile outside the current action messages."
            })
            .to_string();
        }
    }
    let allowed_identity_ids = evidence_messages
        .iter()
        .filter_map(|message| message.identity.as_ref().map(|id| id.0.as_str()))
        .collect::<Vec<_>>();
    if !allowed_identity_ids.is_empty() {
        if explicit_identity_ids
            .iter()
            .any(|id| !allowed_identity_ids.contains(&id.as_str()))
        {
            return json!({
                "error": "Refusing to save memory to an identity outside the current action messages."
            })
            .to_string();
        }
    }

    let mut subjects: Vec<MemorySubject> = if subject_actor {
        vec![MemorySubject::actor(Some("self".into()), 1.0)]
    } else {
        explicit_identity_ids
            .into_iter()
            .map(|id| MemorySubject::identity(IdentityId(id), Some("about".into()), 1.0))
            .collect()
    };
    if !subject_actor {
        subjects.extend(
            explicit_profile_ids
                .into_iter()
                .map(|id| MemorySubject::profile(ProfileId(id), Some("about".into()), 1.0)),
        );
    }
    if subjects.is_empty() {
        if let Some(profile) = source_message.and_then(|message| message.profile.clone()) {
            subjects.push(MemorySubject::profile(profile, Some("about".into()), 1.0));
        } else if let Some(identity) = source_message.and_then(|message| message.identity.clone()) {
            subjects.push(MemorySubject::identity(identity, Some("about".into()), 1.0));
        }
    }

    let content = canonicalize_content_for_subjects(&raw_content, &subjects, ctx).await;
    let source_conversation = source_message
        .map(|m| m.conversation.clone())
        .or_else(|| ctx.conversation.clone())
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()));

    let embedding_result = ctx.router.embed_with_metadata(&[&content]).await.ok();
    let embedding_model = embedding_result.as_ref().map(|result| result.model.clone());
    let embedding = embedding_result.and_then(|mut result| result.embeddings.pop());

    let now = super::super::util::now();
    let next_review_at =
        sensitive_memory_next_review_at(now, &privacy_category, explicit_next_review_at);
    let memory = Memory {
        id: MemoryId(format!("mem-{}", super::super::util::uuid_v4())),
        kind,
        memory_type,
        truth_status,
        content,
        source: source_conversation
            .as_ref()
            .map(|conv| MemorySource::Conversation {
                conversation_id: conv.clone(),
                identity_id: source_message.and_then(|m| m.identity.clone()),
                profile_id: source_message.and_then(|m| m.profile.clone()),
                person_id: source_message.and_then(|m| m.person.clone()),
                message_id: evidence_message_ids
                    .first()
                    .cloned()
                    .or_else(|| source_message.map(|m| m.message_id.clone())),
            })
            .unwrap_or(MemorySource::Reflection),
        importance,
        confidence,
        sensitivity,
        sensitivity_category,
        emotional_valence,
        created_at: now,
        accessed_at: now,
        access_count: 0,
        tags,
        subjects,
        evidence_message_ids,
        evidence_quote,
        evidence,
        expires_at,
        stability,
        supersedes: supersedes.clone(),
        superseded_by: None,
        contradiction_group,
        privacy_category,
        visibility_scope,
        last_confirmed_at,
        next_review_at,
        dedupe_key,
        embedding_model,
        embedding_version: None,
        embedding,
    };

    match ctx.store.store_memory(&memory).await {
        Ok(id) => {
            if id == memory.id {
                ctx.metrics.record_memory_created();
            } else {
                ctx.metrics.record_memory_updated();
            }
            if let Some(superseded) = supersedes.filter(|superseded| superseded != &id) {
                if let Err(e) = ctx
                    .store
                    .update_memory(
                        &superseded,
                        &crate::store::MemoryUpdate {
                            truth_status: Some(TruthStatus::Outdated),
                            superseded_by: Some(id.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    return format!(
                        "Memory saved: {}, but failed to link superseded memory: {e}",
                        id.0
                    );
                }
                ctx.metrics.record_memory_updated();
                ctx.metrics.record_memory_superseded();
            }
            state.memories_formed.push(id.clone());
            format!("Memory saved: {}", id.0)
        }
        Err(e) => format!("Failed to save memory: {e}"),
    }
}

fn evidence_source_messages(ctx: &SessionContext, state: &SessionState) -> Vec<InboundMessage> {
    ctx.messages
        .iter()
        .chain(state.presented_injected_messages.iter())
        .chain(state.presented_read_messages.iter())
        .cloned()
        .collect()
}

fn selected_evidence_messages(
    messages: &[InboundMessage],
    evidence_message_ids: &[String],
) -> Result<Vec<InboundMessage>, Vec<String>> {
    if evidence_message_ids.is_empty() {
        return Ok(messages.to_vec());
    }

    let mut selected = Vec::new();
    let mut missing = Vec::new();
    for id in evidence_message_ids {
        match messages.iter().find(|message| message.message_id == *id) {
            Some(message) => selected.push(message.clone()),
            None => missing.push(id.clone()),
        }
    }
    if missing.is_empty() {
        Ok(selected)
    } else {
        Err(missing)
    }
}

fn source_message_for_evidence<'a>(
    messages: &'a [InboundMessage],
    evidence_message_ids: &[String],
) -> Option<&'a InboundMessage> {
    evidence_message_ids
        .iter()
        .find_map(|id| messages.iter().find(|message| message.message_id == *id))
        .or_else(|| messages.first())
}

fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn clamp_valence(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests;
