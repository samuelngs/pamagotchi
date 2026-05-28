use super::context::{current_identity, current_person, current_profile};
use super::social::person_link_allows_person_level_update;
use crate::core::tools::SessionContext;
use crate::store::{
    DEFAULT_MAX_SENSITIVITY, Memory, MemorySource, MemorySubjectType, PrivacyCategory,
    VisibilityScope,
};
use protocol::{MemoryId, PersonId, ProfileId};
use serde_json::Value;

pub(super) fn sensitive_recall_requested(args: &Value) -> bool {
    args["include_sensitive"].as_bool().unwrap_or(false)
        || args["max_sensitivity"]
            .as_f64()
            .is_some_and(|max| max as f32 > DEFAULT_MAX_SENSITIVITY)
}

pub(super) fn memory_recall_targets_current(args: &Value, ctx: &SessionContext) -> bool {
    if memory_global_recall_requested(args) {
        return false;
    }

    let identity = args["identity"].as_str().filter(|id| !id.is_empty());
    let profile = args["profile"].as_str().filter(|id| !id.is_empty());
    let person = args["person"].as_str().filter(|id| !id.is_empty());
    if identity.is_none() && profile.is_none() && person.is_none() {
        return current_identity(ctx).is_some()
            || current_profile(ctx).is_some()
            || current_person(ctx).is_some();
    }

    identity.is_none_or(|target| current_identity(ctx) == Some(target))
        && profile.is_none_or(|target| current_profile(ctx) == Some(target))
        && person.is_none_or(|target| current_person(ctx) == Some(target))
}

fn memory_global_recall_requested(args: &Value) -> bool {
    matches!(args["scope"].as_str(), Some("global")) || args["global"].as_bool().unwrap_or(false)
}

pub(super) fn identity_memory_write_requested(args: &Value) -> bool {
    let has_identity_tag = args["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .any(is_identity_memory_marker);
    let identity_type = args["memory_type"]
        .as_str()
        .is_some_and(|value| value == "identity_claim");
    let identity_sensitivity = args["sensitivity_category"]
        .as_str()
        .is_some_and(is_identity_memory_marker);
    let actor_subject = args["subject_actor"].as_bool().unwrap_or(false);

    has_identity_tag || identity_type || identity_sensitivity || actor_subject
}

fn is_identity_memory_marker(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "identity" | "identity_claim" | "self" | "name"
    )
}

pub(super) fn memory_is_current_profile_owned_for_forget(
    mem: &Memory,
    ctx: &SessionContext,
) -> bool {
    let current = ctx.messages.first();
    let Some(current_profile) = current.and_then(|msg| msg.profile.as_ref()) else {
        return false;
    };

    if matches!(
        mem.privacy_category,
        PrivacyCategory::Sensitive | PrivacyCategory::Secret
    ) || matches!(
        mem.visibility_scope,
        VisibilityScope::Person | VisibilityScope::ChosenPersonOnly | VisibilityScope::Global
    ) {
        return false;
    }

    for subject in &mem.subjects {
        match subject.subject_type {
            MemorySubjectType::Profile if subject.subject_id == current_profile.0 => {}
            MemorySubjectType::Profile => return false,
            MemorySubjectType::Identity | MemorySubjectType::Person | MemorySubjectType::Actor => {
                return false;
            }
        }
    }

    if mem.subjects.iter().any(|subject| {
        subject.subject_type == MemorySubjectType::Profile
            && subject.subject_id == current_profile.0
    }) {
        return true;
    }

    match &mem.source {
        MemorySource::Conversation { profile_id, .. } => profile_id
            .as_ref()
            .is_some_and(|profile_id| profile_id == current_profile),
        _ => false,
    }
}

pub(super) async fn memory_promotion_target_is_verified(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    let Some(memory_id) = args["memory_id"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    let Some(person_id) = args["person"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    let target = PersonId(person_id.to_string());
    let memory = ctx
        .store
        .get_memory(&MemoryId(memory_id.to_string()))
        .await
        .map_err(|e| format!("Could not verify memory promotion target: {e}"))?;
    let Some(memory) = memory else {
        return Ok(false);
    };

    if memory.subjects.iter().any(|subject| {
        subject.subject_type == MemorySubjectType::Person && subject.subject_id == target.0
    }) {
        return Ok(true);
    }

    for subject in memory
        .subjects
        .iter()
        .filter(|subject| subject.subject_type == MemorySubjectType::Profile)
    {
        let profile = ProfileId(subject.subject_id.clone());
        let link = ctx
            .store
            .get_person_for_profile(&profile)
            .await
            .map_err(|e| format!("Could not verify memory promotion profile link: {e}"))?;
        if let Some((_person, link)) = link {
            if link.person_id == target && person_link_allows_person_level_update(&link) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}
