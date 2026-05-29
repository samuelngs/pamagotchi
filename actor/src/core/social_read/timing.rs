use crate::core::handle::StateHandle;
use crate::core::prompt::context::{GatewayCtx, TimingCtx, TypingCtx};
use crate::core::tools::{SessionContext, TYPING_ACTIVE_SECS};
use crate::store::Store;
use protocol::{ConversationId, InboundMessage, PersonId};
use std::sync::Arc;

pub(crate) async fn build_timing_ctx(
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    person: Option<&PersonId>,
    state: &StateHandle,
    session_ctx: &SessionContext,
    now: chrono::DateTime<chrono::Utc>,
) -> TimingCtx {
    let quiet_hours = state
        .read_config()
        .proactivity
        .quiet_hours_utc
        .as_ref()
        .map(|quiet| {
            if let Some(delay) = quiet.delay_until_end(now) {
                format!(
                    "quiet hours active; ends in {}",
                    duration_words(delay as i64)
                )
            } else {
                format!(
                    "quiet hours configured {:02}:00-{:02}:00 UTC; currently open",
                    quiet.start_hour, quiet.end_hour
                )
            }
        });
    let gateway = fetch_gateway_ctx(store, messages, conversation, session_ctx).await;
    let typing = active_typing_ctx(session_ctx, messages, conversation, now.timestamp());
    let (last_inbound, last_outbound) = if let Some(person) = person {
        let actor = state.read_state();
        if let Some(rel) = actor.bonds.get(person) {
            (
                (rel.last_inbound > 0)
                    .then(|| relative_duration(rel.last_inbound, now.timestamp())),
                (rel.last_outbound > 0)
                    .then(|| relative_duration(rel.last_outbound, now.timestamp())),
            )
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    TimingCtx {
        quiet_hours,
        gateway,
        last_inbound,
        last_outbound,
        typing,
    }
}

fn active_typing_ctx(
    session_ctx: &SessionContext,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    now: i64,
) -> Vec<TypingCtx> {
    let current = messages.first().and_then(|message| {
        let sender = message.sender_external_id()?;
        Some((&message.conversation, message.gateway_id.as_str(), sender))
    });
    let Ok(typing) = session_ctx.typing.read() else {
        return vec![];
    };

    let mut active = typing
        .iter()
        .filter_map(|((conv, gateway_id, sender_external_id), started_at)| {
            if conversation.is_some_and(|current| current != conv) {
                return None;
            }
            let elapsed = now.saturating_sub(*started_at);
            if elapsed > TYPING_ACTIVE_SECS {
                return None;
            }
            let is_current_sender =
                current.is_some_and(|(current_conv, current_gateway, sender)| {
                    current_conv == conv
                        && current_gateway == gateway_id.as_str()
                        && sender == sender_external_id.as_str()
                });
            Some(TypingCtx {
                gateway_id: gateway_id.clone(),
                sender_external_id: sender_external_id.clone(),
                active_for: duration_words(elapsed),
                is_current_sender,
            })
        })
        .collect::<Vec<_>>();
    active.sort_by(|a, b| {
        b.is_current_sender
            .cmp(&a.is_current_sender)
            .then_with(|| a.gateway_id.cmp(&b.gateway_id))
            .then_with(|| a.sender_external_id.cmp(&b.sender_external_id))
    });
    active
}

async fn fetch_gateway_ctx(
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    session_ctx: &SessionContext,
) -> Option<GatewayCtx> {
    let gateway_id = if let Some(conversation) = conversation {
        store
            .list_conversations()
            .await
            .ok()
            .and_then(|summaries| {
                summaries
                    .into_iter()
                    .find(|summary| summary.id == *conversation)
                    .and_then(|summary| summary.gateway_id)
            })
            .or_else(|| {
                messages
                    .iter()
                    .find(|message| &message.conversation == conversation)
                    .map(|message| message.gateway_id.clone())
            })
    } else {
        messages.first().map(|message| message.gateway_id.clone())
    }?;

    let (state, connected) = session_ctx
        .gateway
        .connection_state(&gateway_id)
        .map(|state| {
            (
                format!("{state:?}"),
                session_ctx.gateway.is_connected(&gateway_id),
            )
        })
        .unwrap_or_else(|| ("unregistered".into(), false));
    Some(GatewayCtx {
        id: gateway_id,
        state,
        connected,
    })
}

fn relative_duration(from: i64, to: i64) -> String {
    let secs = (to - from).max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute ago".into()
        } else {
            format!("{m} minutes ago")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour ago".into()
        } else {
            format!("{h} hours ago")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day ago".into()
        } else {
            format!("{d} days ago")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week ago".into()
        } else {
            format!("{w} weeks ago")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month ago".into()
        } else {
            format!("{mo} months ago")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year ago".into()
        } else {
            format!("{y} years ago")
        }
    }
}

fn duration_words(secs: i64) -> String {
    if secs < 60 {
        "less than 1 minute".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute".into()
        } else {
            format!("{m} minutes")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour".into()
        } else {
            format!("{h} hours")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day".into()
        } else {
            format!("{d} days")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week".into()
        } else {
            format!("{w} weeks")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month".into()
        } else {
            format!("{mo} months")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year".into()
        } else {
            format!("{y} years")
        }
    }
}
