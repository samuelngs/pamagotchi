use super::context::current_conversation_id;
use crate::core::ActionKind;
use crate::core::tools::{SessionContext, SessionKind};

pub(super) async fn explicit_scheduled_outreach_target_matches(
    ctx: &SessionContext,
    gateway_id: &str,
    external_id: &str,
) -> Result<bool, String> {
    if !matches!(ctx.kind, SessionKind::Action(ActionKind::Outreach)) {
        return Ok(false);
    }
    let Some(conversation) = current_conversation_id(ctx) else {
        return Ok(false);
    };

    let channel = ctx
        .store
        .channel_for_conversation(&conversation)
        .await
        .map_err(|e| format!("Could not verify scheduled outreach target: {e}"))?;

    Ok(channel.is_some_and(|channel| {
        channel.gateway.0 == gateway_id && channel.external_id == external_id
    }))
}
