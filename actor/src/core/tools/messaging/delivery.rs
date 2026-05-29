use super::*;
use crate::core::tools::util;

pub(super) async fn notify_chosen_human_of_delivery_failure(
    ctx: &SessionContext,
    target_gateway: &str,
    target_id: &str,
    conversation: Option<&ConversationId>,
    content: &str,
    error: &str,
    now: i64,
) -> bool {
    let Some(chosen_human) = chosen_human(ctx) else {
        return false;
    };
    let conversation_label = conversation
        .map(|conversation| conversation.0.as_str())
        .unwrap_or("none");
    let intent = IntentRecord {
        id: format!("intent-{}", util::uuid_v4()),
        kind: "scheduled".into(),
        status: "active".into(),
        task: format!(
            "Review failed outbound delivery from action {}. Target: {target_gateway}:{target_id}. Conversation: {conversation_label}. Message length: {} chars. Error: {error}. Decide whether to retry manually, update gateway setup, or ignore.",
            ctx.action_id.0,
            content.chars().count()
        ),
        person: Some(chosen_human),
        profile: None,
        conversation: None,
        fire_at: Some(now),
        condition: None,
        recurrence: None,
        priority: 100,
        dedupe_key: Some(format!(
            "delivery-failure-review:{}:{target_gateway}:{target_id}",
            ctx.action_id.0
        )),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: None,
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_human_approved: true,
    };
    match ctx.store.create_intent(&intent).await {
        Ok(()) => true,
        Err(e) => {
            warn!(
                action = %ctx.action_id,
                %e,
                "failed to create chosen-human review intent for delivery failure"
            );
            false
        }
    }
}

fn chosen_human(ctx: &SessionContext) -> Option<PersonId> {
    let actor = ctx.state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, relationship)| matches!(relationship.authority, Authority::ChosenHuman))
        .map(|(person, _)| person.clone())
}
