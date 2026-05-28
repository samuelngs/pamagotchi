use super::context::{current_conversation, current_person, current_profile};
use super::social::{
    conversation_has_active_person_context, person_has_active_profile_context,
    profile_has_active_person_context,
};
use crate::core::tools::SessionContext;
use protocol::{ConversationId, PersonId, ProfileId};
use serde_json::Value;

pub(crate) fn intent_requires_chosen_person_approval(args: &Value) -> bool {
    args["requires_chosen_person_approval"]
        .as_bool()
        .unwrap_or(false)
        || args["sensitive"].as_bool().unwrap_or(false)
        || sensitive_outreach_text(args["task"].as_str())
        || sensitive_outreach_text(args["condition"].as_str())
}

pub(super) async fn update_activates_pending_chosen_person_approval_intent(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if args["status"].as_str() != Some("active") {
        return Ok(false);
    }
    let Some(id) = args["intent_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(false);
    };
    let intent = ctx
        .store
        .get_intent(id)
        .await
        .map_err(|e| format!("Could not verify intent chosen-person approval status: {e}"))?;
    Ok(intent.is_some_and(|intent| intent.status == "pending_approval"))
}

pub(crate) async fn intent_targets_current_or_verified(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    intent_targets_current_or_verified_with_keys(args, ctx, "person", "profile", "conversation")
        .await
}

pub(crate) async fn intent_targets_current_or_verified_with_keys(
    args: &Value,
    ctx: &SessionContext,
    person_key: &str,
    profile_key: &str,
    conversation_key: &str,
) -> Result<bool, String> {
    let person = args[person_key].as_str().filter(|id| !id.is_empty());
    let profile = args[profile_key].as_str().filter(|id| !id.is_empty());
    let conversation = args[conversation_key].as_str().filter(|id| !id.is_empty());
    if person.is_none() && profile.is_none() && conversation.is_none() {
        return Ok(true);
    }

    if let Some(person) = person {
        if current_person(ctx) != Some(person)
            && !person_has_active_profile_context(ctx, &PersonId(person.to_string())).await?
        {
            return Ok(false);
        }
    }
    if let Some(profile) = profile {
        if current_profile(ctx) != Some(profile)
            && !profile_has_active_person_context(ctx, &ProfileId(profile.to_string())).await?
        {
            return Ok(false);
        }
    }
    if let Some(conversation) = conversation {
        if current_conversation(ctx) != Some(conversation)
            && !conversation_has_active_person_context(
                ctx,
                &ConversationId(conversation.to_string()),
            )
            .await?
        {
            return Ok(false);
        }
    }

    Ok(true)
}

pub(super) async fn intent_id_targets_current_or_verified(
    id: &str,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if id.is_empty() {
        return Ok(false);
    }
    let intent = ctx
        .store
        .get_intent(id)
        .await
        .map_err(|e| format!("Could not verify intent target: {e}"))?;
    let Some(intent) = intent else {
        return Ok(true);
    };
    let has_target =
        intent.person.is_some() || intent.profile.is_some() || intent.conversation.is_some();
    if !has_target {
        return Ok(false);
    }
    if let Some(person) = intent.person {
        if current_person(ctx) != Some(person.0.as_str())
            && !person_has_active_profile_context(ctx, &person).await?
        {
            return Ok(false);
        }
    }
    if let Some(profile) = intent.profile {
        if current_profile(ctx) != Some(profile.0.as_str())
            && !profile_has_active_person_context(ctx, &profile).await?
        {
            return Ok(false);
        }
    }
    if let Some(conversation) = intent.conversation {
        if current_conversation(ctx) != Some(conversation.0.as_str())
            && !conversation_has_active_person_context(ctx, &conversation).await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sensitive_outreach_text(text: Option<&str>) -> bool {
    let Some(text) = text else {
        return false;
    };
    let text = text.to_ascii_lowercase();
    [
        "password",
        "passcode",
        "token",
        "secret",
        "confidential",
        "private",
        "medical",
        "health",
        "diagnosis",
        "therapy",
        "legal",
        "lawyer",
        "financial",
        "finance",
        "bank",
        "tax",
        "payment",
        "address",
        "social security",
        "ssn",
        "identity",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

pub(super) fn explicit_intent_targets_current(args: &Value, ctx: &SessionContext) -> bool {
    explicit_target_matches(args, "person", current_person(ctx))
        && explicit_target_matches(args, "profile", current_profile(ctx))
        && explicit_target_matches(args, "conversation", current_conversation(ctx))
}

fn explicit_target_matches(args: &Value, key: &str, current: Option<&str>) -> bool {
    let Some(target) = args[key].as_str().filter(|id| !id.is_empty()) else {
        return true;
    };
    current.is_some_and(|current| current == target)
}

pub(super) async fn intent_id_targets_current(
    id: &str,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if id.is_empty() {
        return Ok(false);
    }
    let intent = ctx
        .store
        .get_intent(id)
        .await
        .map_err(|e| format!("Could not verify intent target: {e}"))?;
    let Some(intent) = intent else {
        return Ok(true);
    };

    let has_target =
        intent.person.is_some() || intent.profile.is_some() || intent.conversation.is_some();
    if !has_target {
        return Ok(false);
    }

    if intent
        .person
        .as_ref()
        .is_some_and(|target| current_person(ctx).is_none_or(|current| current != target.0))
    {
        return Ok(false);
    }
    if intent
        .profile
        .as_ref()
        .is_some_and(|target| current_profile(ctx).is_none_or(|current| current != target.0))
    {
        return Ok(false);
    }
    if intent
        .conversation
        .as_ref()
        .is_some_and(|target| current_conversation(ctx).is_none_or(|current| current != target.0))
    {
        return Ok(false);
    }

    Ok(true)
}
