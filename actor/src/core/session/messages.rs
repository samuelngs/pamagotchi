use super::super::prompt;
use super::super::tools::{SessionContext, SessionState};
use crate::store::{MessageRole, StoredMessage};
use base64::Engine;
use inference::{Capability, ContentPart, Message};
use protocol::{MediaAttachment, MediaKind};
use serde_json::Value;
use tracing::warn;

pub(super) async fn build_prompt(ctx: &SessionContext) -> anyhow::Result<String> {
    prompt::build_system_prompt(
        &ctx.state,
        &ctx.store,
        &ctx.kind,
        &ctx.messages,
        ctx.conversation.as_ref(),
        ctx,
        &ctx.authority,
    )
    .await
}

pub(super) async fn ingest_messages(ctx: &SessionContext, llm_messages: &mut Vec<Message>) {
    for inbound in &ctx.messages {
        let display = inbound.display_content();
        llm_messages.push(user_message_for_inbound(ctx, inbound).await);
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: inbound.timestamp,
                role: MessageRole::User,
                content: display,
                identity: inbound.identity.clone(),
                profile: inbound.profile.clone(),
                person: inbound.person.clone(),
                metadata: message_metadata(inbound),
            };
            ctx.store
                .append_message(conv, None, None, &stored)
                .await
                .ok();
        }
    }
}

pub(super) async fn inject_pending_messages(
    ctx: &SessionContext,
    state: &mut SessionState,
    llm_messages: &mut Vec<Message>,
) {
    for msg in &state.injected_messages {
        let display = msg.display_content();
        if llm_messages
            .iter()
            .any(|m| matches!(m, Message::User(u) if u.text_eq(&display)))
        {
            continue;
        }
        llm_messages.push(Message::system(
            "--- New message arrived while you were working. Address it before finishing. ---",
        ));
        llm_messages.push(user_message_for_inbound(ctx, msg).await);
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: msg.timestamp,
                role: MessageRole::User,
                content: display,
                identity: msg.identity.clone(),
                profile: msg.profile.clone(),
                person: msg.person.clone(),
                metadata: message_metadata(msg),
            };
            ctx.store
                .append_message(conv, None, None, &stored)
                .await
                .ok();
        }
    }
}

pub(super) fn resolve_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages.first().and_then(|msg| {
        if msg.gateway_id.is_empty() || msg.external_id.is_empty() {
            None
        } else {
            Some((msg.gateway_id.clone(), msg.external_id.clone()))
        }
    })
}

pub(super) fn required_capabilities(
    messages: &[protocol::InboundMessage],
    injected_messages: &[protocol::InboundMessage],
) -> Vec<Capability> {
    let needs_vision = messages
        .iter()
        .chain(injected_messages.iter())
        .flat_map(|msg| &msg.attachments)
        .any(|attachment| requires_vision(&attachment.kind));

    if needs_vision {
        vec![Capability::Vision]
    } else {
        Vec::new()
    }
}

pub(super) async fn user_message_for_inbound(
    ctx: &SessionContext,
    msg: &protocol::InboundMessage,
) -> Message {
    let display = msg.display_content();
    let mut parts = vec![ContentPart::text(display.clone())];

    for attachment in &msg.attachments {
        if !can_embed_as_vision_input(attachment) {
            continue;
        }

        if let Some(url) = attachment.url.as_deref() {
            if is_model_visible_url(url) {
                parts.push(ContentPart::image_url(url.to_string()));
                continue;
            }
        }

        let Some(asset_id) = attachment.asset_id.as_ref() else {
            continue;
        };
        let Some(media_store) = ctx.media_store.as_ref() else {
            continue;
        };

        match media_store.read_bytes(asset_id) {
            Ok(Some(bytes)) => {
                let mime = attachment
                    .mime
                    .as_deref()
                    .unwrap_or_else(|| default_visual_mime(&attachment.kind));
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                parts.push(ContentPart::image_url(format!(
                    "data:{mime};base64,{encoded}"
                )));
            }
            Ok(None) => {
                warn!(asset_id = %asset_id.0, "media asset missing for vision input");
            }
            Err(e) => {
                warn!(%e, asset_id = %asset_id.0, "failed to read media asset for vision input");
            }
        }
    }

    if parts.len() > 1 {
        Message::user_content(parts)
    } else {
        Message::user(display)
    }
}

fn requires_vision(kind: &MediaKind) -> bool {
    matches!(
        kind,
        MediaKind::Image | MediaKind::Video | MediaKind::Sticker
    )
}

fn can_embed_as_vision_input(attachment: &MediaAttachment) -> bool {
    matches!(attachment.kind, MediaKind::Image | MediaKind::Sticker)
        || attachment
            .mime
            .as_deref()
            .is_some_and(|mime| mime.starts_with("image/"))
}

fn is_model_visible_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:image/")
}

fn default_visual_mime(kind: &MediaKind) -> &'static str {
    match kind {
        MediaKind::Sticker => "image/webp",
        _ => "image/png",
    }
}

pub(super) fn message_metadata(msg: &protocol::InboundMessage) -> Value {
    let mut metadata = msg.metadata.clone();
    if msg.attachments.is_empty() {
        return metadata;
    }

    let attachments_value = serde_json::to_value(&msg.attachments).unwrap_or(Value::Null);
    match &mut metadata {
        Value::Object(obj) => {
            obj.insert("attachments".into(), attachments_value);
            metadata
        }
        Value::Null => serde_json::json!({ "attachments": attachments_value }),
        other => serde_json::json!({
            "source_metadata": other.clone(),
            "attachments": attachments_value,
        }),
    }
}
