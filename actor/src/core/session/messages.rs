use super::super::prompt;
use super::super::tools::{SessionContext, SessionState};
use crate::store::{ActionMessageRecord, MessageRole, StoredMessage};
use base64::Engine;
use inference::{Capability, ContentPart, Message};
use protocol::{MediaAttachment, MediaKind};
use serde_json::Value;
use std::collections::HashSet;
use tracing::warn;

pub(super) async fn build_prompt(ctx: &SessionContext) -> anyhow::Result<String> {
    prompt::build_system_prompt(
        &ctx.state,
        &ctx.store,
        &ctx.kind,
        &ctx.messages,
        ctx.conversation.as_ref(),
        ctx,
        &ctx.relationship_standing,
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
                source_gateway_id: Some(inbound.gateway_id.clone()),
                source_message_id: Some(inbound.message_id.clone()),
                sender_external_id: inbound.sender_external_id().map(str::to_string),
                reply_external_id: Some(inbound.channel_external_id().to_string()),
                metadata: message_metadata(inbound),
            };
            ctx.store.append_message(conv, &stored).await.ok();
        }
    }
}

pub(super) async fn inject_pending_messages(
    ctx: &SessionContext,
    state: &mut SessionState,
    llm_messages: &mut Vec<Message>,
) {
    let pending = std::mem::take(&mut state.pending_injected_messages);
    for msg in pending {
        let key = inbound_message_key(&msg);
        if key
            .as_ref()
            .is_some_and(|key| state.source_message_keys.contains(key))
        {
            ctx.metrics.record_duplicate_message_suppression();
            continue;
        }
        if key
            .as_ref()
            .is_some_and(|key| state.presented_injected_message_keys.contains(key))
        {
            ctx.metrics.record_duplicate_message_suppression();
            continue;
        }
        if let Some(key) = key.as_ref() {
            state.queued_injected_message_keys.insert(key.clone());
        }
        if key.as_ref().is_none_or(|key| {
            !state
                .injected_messages
                .iter()
                .any(|existing| inbound_message_key(existing).as_ref() == Some(key))
        }) {
            state.injected_messages.push(msg.clone());
        }

        let display = msg.display_content();
        llm_messages.push(Message::system(
            "--- New message arrived while you were working. Address it before finishing. ---",
        ));
        llm_messages.push(user_message_for_inbound(ctx, &msg).await);
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: msg.timestamp,
                role: MessageRole::User,
                content: display.clone(),
                identity: msg.identity.clone(),
                profile: msg.profile.clone(),
                person: msg.person.clone(),
                source_gateway_id: Some(msg.gateway_id.clone()),
                source_message_id: Some(msg.message_id.clone()),
                sender_external_id: msg.sender_external_id().map(str::to_string),
                reply_external_id: Some(msg.channel_external_id().to_string()),
                metadata: message_metadata(&msg),
            };
            ctx.store.append_message(conv, &stored).await.ok();
        }
        let record = ActionMessageRecord {
            action_id: ctx.action_id.0.clone(),
            role: "user".into(),
            conversation: Some(msg.conversation.clone()),
            source_gateway_id: Some(msg.gateway_id.clone()),
            source_message_id: Some(msg.message_id.clone()),
            sender_external_id: msg.sender_external_id().map(str::to_string),
            reply_external_id: Some(msg.channel_external_id().to_string()),
            content: Some(display),
            created_at: msg.timestamp,
        };
        if let Err(e) = ctx.store.append_action_message(&record).await {
            warn!(
                action = %ctx.action_id,
                %e,
                "failed to persist injected action message link"
            );
        }
        if let Some(key) = key {
            state.presented_injected_message_keys.insert(key);
        }
        state.presented_injected_messages.push(msg);
        state.presented_injection_count += 1;
    }
}

pub(super) fn inbound_message_key(msg: &protocol::InboundMessage) -> Option<String> {
    if msg.gateway_id.is_empty() || msg.message_id.is_empty() {
        return None;
    }
    Some(format!("{}:{}", msg.gateway_id, msg.message_id))
}

pub(super) fn source_message_keys(messages: &[protocol::InboundMessage]) -> HashSet<String> {
    messages.iter().filter_map(inbound_message_key).collect()
}

pub(super) fn remember_injected_message(
    state: &mut SessionState,
    msg: protocol::InboundMessage,
) -> bool {
    if let Some(key) = inbound_message_key(&msg) {
        if state.source_message_keys.contains(&key)
            || state.queued_injected_message_keys.contains(&key)
        {
            return false;
        }
        state.queued_injected_message_keys.insert(key);
    }
    state.pending_injected_messages.push(msg.clone());
    state.injected_messages.push(msg);
    true
}

pub(super) fn resolve_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages
        .first()
        .and_then(|msg| msg.reply_target())
        .map(|(gateway, target)| (gateway.to_string(), target.to_string()))
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
    let source = serde_json::json!({
        "message_id": msg.message_id,
        "gateway_id": msg.gateway_id,
        "sender": &msg.sender,
        "sender_display_name": msg.sender_display_name(),
        "channel": &msg.channel,
        "channel_external_id": msg.channel_external_id(),
        "legacy_group_id": msg.legacy_group_id().map(|group| group.0),
    });

    let attachments_value = if msg.attachments.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&msg.attachments).unwrap_or(Value::Null))
    };

    match &mut metadata {
        Value::Object(obj) => {
            obj.insert("source".into(), source);
            if let Some(attachments) = attachments_value {
                obj.insert("attachments".into(), attachments);
            }
            metadata
        }
        Value::Null => {
            let mut obj = serde_json::json!({ "source": source });
            if let Some(attachments) = attachments_value {
                obj["attachments"] = attachments;
            }
            obj
        }
        other => {
            let mut obj = serde_json::json!({
                "source_metadata": other.clone(),
                "source": source,
            });
            if let Some(attachments) = attachments_value {
                obj["attachments"] = attachments;
            }
            obj
        }
    }
}
