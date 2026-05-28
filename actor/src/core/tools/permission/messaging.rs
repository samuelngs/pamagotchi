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

    let conversation_gateway = ctx
        .store
        .list_conversations()
        .await
        .map_err(|e| format!("Could not verify scheduled outreach target: {e}"))?
        .into_iter()
        .find(|summary| summary.id == conversation)
        .and_then(|summary| summary.gateway_id);
    let messages = ctx
        .store
        .get_messages(&conversation, 20, None)
        .await
        .map_err(|e| format!("Could not verify scheduled outreach target: {e}"))?;

    Ok(messages.iter().rev().any(|message| {
        let Some(reply_external_id) = message.reply_external_id.as_deref() else {
            return false;
        };
        let Some(reply_gateway_id) = message
            .source_gateway_id
            .as_deref()
            .or(conversation_gateway.as_deref())
        else {
            return false;
        };
        reply_gateway_id == gateway_id && reply_external_id == external_id
    }))
}
