use super::*;
use crate::core::tools::util;

pub(super) async fn wait_if_current_sender_is_typing(ctx: &SessionContext) {
    let Some(key) = current_sender_typing_key(ctx) else {
        return;
    };
    if !typing_key_is_active(ctx, &key) {
        return;
    }

    let started = Instant::now();
    loop {
        if !typing_key_is_active(ctx, &key) {
            return;
        }
        if started.elapsed() >= Duration::from_millis(TYPING_SEND_WAIT_MAX_MS) {
            warn!(
                conversation = %key.0.0,
                gateway = %key.1,
                sender_external_id = %key.2,
                "sending despite active typing after bounded wait"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(TYPING_SEND_POLL_MS)).await;
    }
}

fn current_sender_typing_key(ctx: &SessionContext) -> Option<TypingStateKey> {
    let msg = ctx.messages.first()?;
    let sender_external_id = msg.sender_external_id()?;
    Some((
        msg.conversation.clone(),
        msg.gateway_id.clone(),
        sender_external_id.to_string(),
    ))
}

fn typing_key_is_active(ctx: &SessionContext, key: &TypingStateKey) -> bool {
    ctx.typing.read().ok().is_some_and(|typing| {
        typing
            .get(key)
            .is_some_and(|started_at| util::now() - started_at <= TYPING_ACTIVE_SECS)
    })
}
