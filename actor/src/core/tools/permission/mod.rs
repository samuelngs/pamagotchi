use super::SessionKind;
use super::context::SessionContext;
mod context;
mod intent;
mod memory;
mod messaging;
mod social;

use crate::core::ActionKind;
use crate::state::Authority;
use crate::store::MemorySource;
use context::{
    current_conversation, current_person, privileged_conversation_read, privileged_intent_create,
    privileged_intent_write, privileged_memory_recall, privileged_profile_write,
    privileged_sensitive_recall,
};
use intent::{
    explicit_intent_targets_current, intent_id_targets_current,
    intent_id_targets_current_or_verified, update_activates_pending_chosen_person_approval_intent,
};
pub(crate) use intent::{
    intent_requires_chosen_person_approval, intent_targets_current_or_verified,
    intent_targets_current_or_verified_with_keys,
};
use memory::{
    identity_memory_write_requested, memory_is_current_profile_owned_for_forget,
    memory_promotion_target_is_verified, memory_recall_targets_current, sensitive_recall_requested,
};
use messaging::explicit_scheduled_outreach_target_matches;
use protocol::MemoryId;
use serde_json::Value;
pub(crate) use social::{
    person_has_verified_or_strong_profile_context, relationship_trust_ceiling,
    social_relation_targets_current_or_verified,
};

pub(crate) const STRONG_LIKELY_PERSON_LINK_CONFIDENCE: f32 = 0.75;

pub async fn check(name: &str, args: &Value, ctx: &SessionContext) -> Result<(), String> {
    match name {
        "send_message"
            if matches!(
                ctx.kind,
                SessionKind::Action(
                    ActionKind::Review
                        | ActionKind::Research
                        | ActionKind::Consolidate
                        | ActionKind::Ruminate
                )
            ) =>
        {
            return Err(
                "This action is internal/background only; do not send a visible message.".into(),
            );
        }
        "send_message" => {
            let gateway_id = args["gateway_id"].as_str();
            let external_id = args["external_id"].as_str();
            if gateway_id.is_some() != external_id.is_some() {
                return Err(
                    "Explicit outbound sends must include both gateway_id and external_id.".into(),
                );
            }
            if let (Some(gateway_id), Some(external_id)) = (gateway_id, external_id) {
                let current_reply_target = ctx
                    .messages
                    .first()
                    .and_then(|msg| msg.reply_target())
                    .map(|(gateway, target)| (gateway.to_string(), target.to_string()));
                let is_current_reply_target =
                    current_reply_target
                        .as_ref()
                        .is_some_and(|(gateway, target)| {
                            gateway == gateway_id && target == external_id
                        });
                let has_outbound_authority =
                    matches!(ctx.authority, Authority::ChosenPerson | Authority::Trusted);
                let is_scheduled_outreach_target =
                    explicit_scheduled_outreach_target_matches(ctx, gateway_id, external_id)
                        .await?;
                if !is_current_reply_target
                    && !has_outbound_authority
                    && !is_scheduled_outreach_target
                {
                    return Err(
                        "Explicit outbound messaging requires chosen-person/trusted authority or the scheduled outreach target."
                            .into(),
                    );
                }
            }
        }
        "update_profile" => {
            if !privileged_profile_write(ctx) {
                let target = args["ref"].as_str().filter(|id| !id.is_empty());
                let current = ctx
                    .messages
                    .first()
                    .and_then(|message| message.profile.as_ref());
                if target.is_some_and(|target| current.is_none_or(|current| current.0 != target)) {
                    return Err(
                        "Updating another profile requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "update_person" => {
            if !privileged_profile_write(ctx) {
                let target = args["ref"].as_str().filter(|id| !id.is_empty());
                let current = ctx
                    .messages
                    .first()
                    .and_then(|message| message.person.as_ref());
                if target.is_some_and(|target| current.is_none_or(|current| current.0 != target)) {
                    return Err(
                        "Updating another person requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "update_conversation_summary" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && !matches!(
                    ctx.kind,
                    SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                )
            {
                let target = args["conversation"].as_str().filter(|id| !id.is_empty());
                if target.is_some_and(|target| {
                    current_conversation(ctx).is_none_or(|current| current != target)
                }) {
                    return Err(
                        "Updating another conversation summary requires chosen-person authority or review/consolidation context."
                            .into(),
                    );
                }
            }
        }
        "upsert_social_relation" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && !matches!(
                    ctx.kind,
                    SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                )
            {
                return Err(
                    "Social graph updates require chosen-person authority or review context."
                        .into(),
                );
            }
            if args["source_kind"].as_str() == Some("chosen_person_confirmed")
                && !matches!(ctx.authority, Authority::ChosenPerson)
            {
                return Err(
                    "Chosen-person-confirmed social relations require chosen-person authority."
                        .into(),
                );
            }
            if !social_relation_targets_current_or_verified(args, ctx).await? {
                return Err(
                    "Social graph updates from review must include the current strongly verified person, or chosen-person authority."
                        .into(),
                );
            }
        }
        "form_memory" => {
            if identity_memory_write_requested(args)
                && !matches!(ctx.authority, Authority::ChosenPerson)
            {
                return Err("Something feels wrong about this. You don't want to change something this core about yourself.".into());
            }
        }
        "recall_memories" => {
            if sensitive_recall_requested(args) && !privileged_sensitive_recall(ctx) {
                return Err(
                    "Sensitive memory recall requires chosen-person authority or internal review context."
                        .into(),
                );
            }
            if !privileged_memory_recall(ctx) && !memory_recall_targets_current(args, ctx) {
                return Err(
                    "Reading memories outside the current identity, profile, or person requires chosen-person authority or internal review context."
                        .into(),
                );
            }
        }
        "read_messages" => {
            if !privileged_conversation_read(ctx) {
                let target = args["conversation"].as_str().filter(|id| !id.is_empty());
                if target.is_none() && current_conversation(ctx).is_none() {
                    return Err(
                        "Reading recent conversations without a current conversation requires chosen-person authority or internal review/consolidation/rumination context."
                            .into(),
                    );
                }
                if target.is_some_and(|target| {
                    current_conversation(ctx).is_none_or(|current| current != target)
                }) {
                    return Err(
                        "Reading another conversation requires chosen-person authority or review/consolidation/rumination context."
                            .into(),
                    );
                }
            }
        }
        "inspect_memory" | "delete_memory" => {
            if !matches!(ctx.authority, Authority::ChosenPerson) {
                return Err(
                    "Chosen-person authority is required to inspect or delete memories by id."
                        .into(),
                );
            }
        }
        "forget_memory" => {
            let id = args["memory_id"].as_str().unwrap_or("");
            if let Ok(Some(mem)) = ctx.store.get_memory(&MemoryId(id.to_string())).await {
                if matches!(ctx.authority, Authority::ChosenPerson)
                    || matches!(
                        ctx.kind,
                        SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                    )
                {
                    return Ok(());
                }
                if matches!(mem.source, MemorySource::External) {
                    return Err(
                        "This memory feels fundamental — you instinctively hold onto it.".into(),
                    );
                }
                if !memory_is_current_profile_owned_for_forget(&mem, ctx) {
                    return Err(
                        "Forgetting memories outside the current profile requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "promote_profile_memory_to_person" => {
            if matches!(ctx.authority, Authority::ChosenPerson) {
                return Ok(());
            }
            if !matches!(
                ctx.kind,
                SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
            ) {
                return Err(
                    "Promoting profile memories to person-level memories requires chosen-person authority or internal review."
                        .into(),
                );
            }
            if !memory_promotion_target_is_verified(args, ctx).await? {
                return Err(
                    "Promoting a profile memory to a person requires a verified or strong likely link between a memory profile subject and that person."
                        .into(),
                );
            }
        }
        "demote_person_memory_to_profile" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && !matches!(
                    ctx.kind,
                    SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                )
            {
                return Err(
                    "Demoting person-level memories requires chosen-person authority or internal review."
                        .into(),
                );
            }
        }
        "create_intent" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && privileged_intent_create(ctx)
                && !intent_targets_current_or_verified(args, ctx).await?
            {
                return Err(
                    "Third-party proactive outreach requires chosen-person authority or a verified target profile."
                        .into(),
                );
            }
            if !privileged_intent_create(ctx) && !explicit_intent_targets_current(args, ctx) {
                return Err(
                    "Creating intents for another person, profile, or conversation requires chosen-person authority or internal context."
                        .into(),
                );
            }
        }
        "update_intent" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && update_activates_pending_chosen_person_approval_intent(args, ctx).await?
            {
                return Err(
                    "Activating an chosen-person-approval intent requires chosen-person authority."
                        .into(),
                );
            }
            if intent_requires_chosen_person_approval(args)
                && !matches!(ctx.authority, Authority::ChosenPerson)
            {
                return Err(
                    "Sensitive proactive outreach requires chosen-person approval before an intent is updated."
                        .into(),
                );
            }
            if !matches!(ctx.authority, Authority::ChosenPerson) && privileged_intent_write(ctx) {
                let id = args["intent_id"].as_str().unwrap_or("");
                if !intent_id_targets_current_or_verified(id, ctx).await?
                    || !intent_targets_current_or_verified(args, ctx).await?
                {
                    return Err(
                        "Third-party proactive outreach requires chosen-person authority or a verified target profile."
                            .into(),
                    );
                }
            }
            if !privileged_intent_write(ctx) {
                let id = args["intent_id"].as_str().unwrap_or("");
                if !intent_id_targets_current(id, ctx).await? {
                    return Err(
                        "Updating intents for another person, profile, or conversation requires chosen-person authority or review context."
                            .into(),
                    );
                }
                if !explicit_intent_targets_current(args, ctx) {
                    return Err(
                        "Retargeting intents to another person, profile, or conversation requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "delete_intent" => {
            if !privileged_intent_write(ctx) {
                let id = args["intent_id"].as_str().unwrap_or("");
                if !intent_id_targets_current(id, ctx).await? {
                    return Err(
                        "Cancelling intents for another person, profile, or conversation requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "apply_review" => {
            if !matches!(ctx.kind, SessionKind::Action(ActionKind::Review)) {
                return Err(
                    "Structured review application is allowed only in review actions.".into(),
                );
            }
        }
        "reflect" => {
            if let Some(rels) = args["relationship_changes"].as_array() {
                for r in rels {
                    if r.get("authority").is_some()
                        && !matches!(ctx.authority, Authority::ChosenPerson)
                    {
                        return Err(
                            "Changing how you feel about someone isn't something you'd do on command."
                                .into(),
                        );
                    }
                    if !privileged_profile_write(ctx) {
                        let target = r["person"].as_str().filter(|id| !id.is_empty());
                        if target.is_some_and(|target| {
                            current_person(ctx).is_none_or(|current| current != target)
                        }) {
                            return Err(
                                "Reflecting on relationship changes for another person requires chosen-person authority or review context."
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
