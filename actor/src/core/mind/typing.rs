use super::*;

pub(super) fn typing_deferred_message_matches(
    msg: &InboundMessage,
    conversation: &protocol::ConversationId,
    gateway_id: &str,
    sender_external_id: &str,
) -> bool {
    evaluate::defer_reason(msg) == Some("typing")
        && &msg.conversation == conversation
        && msg.gateway_id.as_str() == gateway_id
        && msg.sender_external_id() == Some(sender_external_id)
}

pub(super) fn pending_typing_message_error(prior: Option<&str>, error: String) -> String {
    match prior {
        Some(prior) if !prior.is_empty() => format!("{prior}; {error}"),
        _ => error,
    }
}
