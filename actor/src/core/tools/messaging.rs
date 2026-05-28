use super::context::{
    SessionContext, SessionKind, SessionState, TYPING_ACTIVE_SECS, TypingStateKey,
};
use crate::core::ActionKind;
use crate::state::{Authority, RelationshipChange, RelationshipInteraction};
use crate::store::{
    ActionMessageRecord, IntentRecord, MessageRole, OutboundDeliveryRecord, StoredMessage,
};
use inference::Tool;
use protocol::{
    ConversationId, GroupId, InboundMessage, MediaAssetId, MediaAttachment, MediaKind, PersonId,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tracing::warn;

const TYPING_SEND_WAIT_MAX_MS: u64 = 1_500;
const TYPING_SEND_POLL_MS: u64 = 100;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "send_message".into(),
            description: "Send a message. Omit gateway_id and external_id to reply in the current conversation. Provide both to send to a specific destination (use get_person with include_identities=true to find allowed gateway identities).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The message text"
                    },
                    "gateway_id": {
                        "type": "string",
                        "description": "Gateway to send through (e.g. discord, telegram, whatsapp)"
                    },
                    "external_id": {
                        "type": "string",
                        "description": "Recipient's ID on that gateway. Must be paired with gateway_id."
                    },
                    "media_url": {
                        "type": "string",
                        "description": "URL of media to attach. Some gateways require media_asset_id instead."
                    },
                    "media_asset_id": {
                        "type": "string",
                        "description": "Stored media asset ID to attach. Required for WhatsApp media sends."
                    },
                    "media_type": {
                        "type": "string",
                        "enum": ["image", "video", "audio", "sticker", "file"],
                        "description": "Type of media attachment"
                    },
                    "mime_type": {
                        "type": "string",
                        "description": "MIME type of the media (e.g. image/png, video/mp4)"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Filename for file attachments"
                    },
                    "attachments": {
                        "type": "array",
                        "description": "Media attachments to send. Use media_asset_id for stored assets, especially WhatsApp.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "media_asset_id": {
                                    "type": "string",
                                    "description": "Stored media asset ID to attach"
                                },
                                "media_url": {
                                    "type": "string",
                                    "description": "URL of media to attach for gateways that support URL attachments"
                                },
                                "media_type": {
                                    "type": "string",
                                    "enum": ["image", "video", "audio", "sticker", "file"],
                                    "description": "Type of media attachment"
                                },
                                "mime_type": {
                                    "type": "string",
                                    "description": "MIME type of the media"
                                },
                                "filename": {
                                    "type": "string",
                                    "description": "Filename for file attachments"
                                }
                            },
                            "required": ["media_type"]
                        }
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "read_messages".into(),
            description: "Read messages from a conversation. Use to access older history beyond what's in your current context. Internal background actions may omit conversation to read a bounded recent-conversation view.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID. Defaults to current conversation."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max messages to return (default 10)",
                        "default": 10
                    },
                    "before": {
                        "type": "integer",
                        "description": "Unix timestamp. Only return messages before this time. Use to page backwards through history."
                    }
                }
            }),
        },
        Tool {
            name: "update_conversation_summary".into(),
            description: "Update the rolling summary for the current conversation. Use during review or consolidation after reading enough recent messages.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID. Defaults to current conversation."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Compact summary preserving important facts, decisions, open questions, commitments, emotional tone, and last visible response."
                    },
                    "covered_message_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Message ids covered by this summary. Use the top-level message_id values returned by read_messages. Defaults to the current action's messages."
                    }
                },
                "required": ["summary"]
            }),
        },
    ]
}

fn current_conversation(ctx: &SessionContext) -> Option<ConversationId> {
    ctx.conversation
        .clone()
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()))
}

fn current_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages
        .first()
        .and_then(|msg| msg.reply_target())
        .map(|(gateway, target)| (gateway.to_string(), target.to_string()))
}

async fn default_delivery_target(ctx: &SessionContext) -> Option<(String, String)> {
    if let Some((gateway, target)) = ctx.messages.first().and_then(|msg| msg.reply_target()) {
        return Some((gateway.to_string(), target.to_string()));
    }

    let conversation = current_conversation(ctx)?;
    let conversation_gateway =
        ctx.store
            .list_conversations()
            .await
            .ok()
            .and_then(|conversations| {
                conversations
                    .into_iter()
                    .find(|summary| summary.id == conversation)
                    .and_then(|summary| summary.gateway_id)
            });
    let messages = ctx.store.get_messages(&conversation, 20, None).await.ok()?;

    messages.iter().rev().find_map(|message| {
        let target = message.reply_external_id.as_ref()?;
        let gateway = message
            .source_gateway_id
            .as_ref()
            .or(conversation_gateway.as_ref())?;
        Some((gateway.clone(), target.clone()))
    })
}

async fn outbound_relationship_person(
    ctx: &SessionContext,
    conversation: Option<&ConversationId>,
) -> Option<PersonId> {
    if let Some(person) = ctx.messages.first().and_then(|msg| msg.person.clone()) {
        return Some(person);
    }

    let conversation = conversation?;
    ctx.store
        .list_conversations()
        .await
        .ok()?
        .into_iter()
        .find(|summary| summary.id == *conversation)
        .and_then(|summary| summary.person)
}

pub async fn send(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let content = args["content"].as_str().unwrap_or("").to_string();
    let gateway_id = args["gateway_id"].as_str();
    let external_id = args["external_id"].as_str();

    let attachments = match parse_attachments(args) {
        Ok(attachments) => attachments,
        Err(message) => return message,
    };

    let is_outbound = gateway_id.is_some() && external_id.is_some();

    let (target_gateway, target_id) = if is_outbound {
        (
            gateway_id.unwrap().to_string(),
            external_id.unwrap().to_string(),
        )
    } else if let Some((gateway, target)) = default_delivery_target(ctx).await {
        (gateway, target)
    } else {
        return "No delivery target — message not sent.".into();
    };

    let conversation = current_conversation(ctx);
    if !is_outbound {
        wait_if_current_sender_is_typing(ctx).await;
    }
    state.attempted_send = true;
    let delivery = ctx
        .gateway
        .send_message(&target_gateway, &target_id, &content, &attachments)
        .await;

    if !state.composing_released
        && current_composing_target(ctx)
            .as_ref()
            .is_some_and(|(gateway, id)| gateway == &target_gateway && id == &target_id)
    {
        ctx.gateway
            .release_composing(&target_gateway, &target_id)
            .await;
        state.composing_released = true;
    }

    let attempted_at = super::util::now();
    let action_message = ActionMessageRecord {
        action_id: ctx.action_id.0.clone(),
        role: if delivery.is_ok() {
            "assistant".into()
        } else {
            "assistant_delivery_failed".into()
        },
        conversation: conversation.clone(),
        source_gateway_id: None,
        source_message_id: None,
        sender_external_id: None,
        reply_external_id: Some(target_id.clone()),
        content: Some(content.clone()),
        created_at: attempted_at,
    };
    if let Err(e) = ctx.store.append_action_message(&action_message).await {
        warn!(
            action = %ctx.action_id,
            %e,
            "failed to persist assistant action message link"
        );
    }

    if delivery.is_ok() {
        if let Some(conv) = conversation.clone() {
            let stored = StoredMessage {
                timestamp: attempted_at,
                role: MessageRole::Assistant,
                content: content.clone(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some(target_id.clone()),
                metadata: outbound_metadata(&attachments),
            };
            ctx.store
                .append_message(&conv, Some(&target_gateway), None, &stored)
                .await
                .ok();
        }
    }

    let (delivery_status, delivery_error) = match &delivery {
        Ok(_) => ("delivered", None),
        Err(e) => ("failed", Some(e.to_string())),
    };
    ctx.metrics.record_outbound_delivery(delivery.is_ok());
    if let Err(e) = ctx
        .store
        .append_outbound_delivery(&OutboundDeliveryRecord {
            action_id: ctx.action_id.0.clone(),
            conversation: conversation.clone(),
            gateway_id: target_gateway.clone(),
            external_id: target_id.clone(),
            status: delivery_status.into(),
            error: delivery_error,
            attempted_at,
        })
        .await
    {
        warn!(
            action = %ctx.action_id,
            %e,
            "failed to persist outbound delivery status"
        );
    }

    match delivery {
        Ok(_) => {
            state.responded = true;
            if let Some(person) = outbound_relationship_person(ctx, conversation.as_ref()).await {
                let interaction = if matches!(ctx.kind, SessionKind::Action(ActionKind::Outreach)) {
                    RelationshipInteraction::ProactiveOutbound
                } else {
                    RelationshipInteraction::Outbound
                };
                state.delta.relationship_changes.push(RelationshipChange {
                    person,
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.002,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: Some(interaction),
                });
            }
            if is_outbound {
                format!("Message sent to {target_gateway}:{target_id}.")
            } else {
                "Message sent.".into()
            }
        }
        Err(e) => {
            let error = e.to_string();
            let supervisor_notified = notify_chosen_person_of_delivery_failure(
                ctx,
                &target_gateway,
                &target_id,
                conversation.as_ref(),
                &content,
                &error,
                attempted_at,
            )
            .await;
            warn!(
                action = %ctx.action_id,
                error = %error,
                gateway = %target_gateway,
                "message delivery failed"
            );
            if supervisor_notified {
                format!(
                    "Delivery failed; message was not added to visible conversation history and is not marked delivered. Chosen-person review is queued: {error}"
                )
            } else {
                format!(
                    "Delivery failed; message was not added to visible conversation history and is not marked delivered: {error}"
                )
            }
        }
    }
}

#[cfg(test)]
pub async fn read(args: &Value, ctx: &SessionContext) -> String {
    read_inner(args, ctx, None).await
}

pub async fn read_with_state(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> String {
    read_inner(args, ctx, Some(state)).await
}

async fn read_inner(
    args: &Value,
    ctx: &SessionContext,
    mut state: Option<&mut SessionState>,
) -> String {
    let conv = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| current_conversation(ctx));

    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let before = args["before"].as_i64();
    let Some(conv) = conv else {
        if can_read_recent_without_current_conversation(ctx) {
            return read_recent_conversations(ctx, state.as_deref_mut(), limit, before).await;
        }
        return "No conversation specified and no current conversation.".into();
    };

    read_conversation_messages(ctx, state.as_deref_mut(), &conv, limit, before).await
}

async fn read_conversation_messages(
    ctx: &SessionContext,
    state: Option<&mut SessionState>,
    conv: &ConversationId,
    limit: usize,
    before: Option<i64>,
) -> String {
    match ctx.store.get_messages(conv, limit, before).await {
        Ok(messages) if messages.is_empty() => json!({"messages": []}).to_string(),
        Ok(messages) => {
            let (gateway_hint, group) = conversation_evidence_context(ctx, conv).await;
            record_read_evidence(
                state,
                &ctx.messages,
                conv,
                gateway_hint.as_deref(),
                group.as_ref(),
                &messages,
            );
            let mut items = Vec::new();
            for message in &messages {
                items.push(message_json(ctx, message).await);
            }
            json!({"messages": items}).to_string()
        }
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}

async fn read_recent_conversations(
    ctx: &SessionContext,
    mut state: Option<&mut SessionState>,
    limit: usize,
    before: Option<i64>,
) -> String {
    let mut remaining = limit.max(1).min(20);
    let conversations = match ctx.store.list_conversations().await {
        Ok(conversations) => conversations,
        Err(e) => return json!({"error": format!("{e}")}).to_string(),
    };

    let mut out = Vec::new();
    for conversation in conversations.into_iter().take(8) {
        if remaining == 0 {
            break;
        }
        let read_limit = remaining.min(5);
        let Ok(messages) = ctx
            .store
            .get_messages(&conversation.id, read_limit, before)
            .await
        else {
            continue;
        };
        if messages.is_empty() {
            continue;
        }
        record_read_evidence(
            state.as_deref_mut(),
            &ctx.messages,
            &conversation.id,
            conversation.gateway_id.as_deref(),
            conversation.group.as_ref(),
            &messages,
        );
        remaining = remaining.saturating_sub(messages.len());
        let mut items = Vec::new();
        for message in &messages {
            items.push(message_json(ctx, message).await);
        }
        out.push(json!({
            "conversation": conversation.id.0,
            "gateway_id": conversation.gateway_id,
            "group": conversation.group.map(|group| group.0),
            "summary": conversation.summary,
            "messages": items,
        }));
    }

    json!({"conversations": out}).to_string()
}

async fn conversation_evidence_context(
    ctx: &SessionContext,
    conv: &ConversationId,
) -> (Option<String>, Option<GroupId>) {
    ctx.store
        .list_conversations()
        .await
        .ok()
        .and_then(|conversations| {
            conversations
                .into_iter()
                .find(|summary| summary.id == *conv)
        })
        .map(|summary| (summary.gateway_id, summary.group))
        .unwrap_or((None, None))
}

fn record_read_evidence(
    state: Option<&mut SessionState>,
    current_messages: &[InboundMessage],
    conversation: &ConversationId,
    gateway_hint: Option<&str>,
    group: Option<&GroupId>,
    messages: &[StoredMessage],
) {
    let Some(state) = state else {
        return;
    };

    for message in messages {
        let evidence = read_evidence_message(conversation, gateway_hint, group, message);
        let id = evidence.message_id.as_str();
        let already_presented = state
            .presented_read_messages
            .iter()
            .chain(state.presented_injected_messages.iter())
            .chain(current_messages.iter())
            .any(|message| message.message_id == id);
        if !already_presented {
            state.presented_read_messages.push(evidence);
        }
    }
}

fn read_evidence_message(
    conversation: &ConversationId,
    gateway_hint: Option<&str>,
    group: Option<&GroupId>,
    message: &StoredMessage,
) -> InboundMessage {
    InboundMessage {
        message_id: message.readable_message_id(),
        gateway_id: message
            .source_gateway_id
            .clone()
            .or_else(|| gateway_hint.map(str::to_string))
            .unwrap_or_default(),
        sender_external_id: message.sender_external_id.clone().unwrap_or_default(),
        sender_display_name: None,
        reply_external_id: message.reply_external_id.clone().unwrap_or_default(),
        conversation: conversation.clone(),
        group: group.cloned(),
        identity: message.identity.clone(),
        profile: message.profile.clone(),
        person: message.person.clone(),
        content: message.content.clone(),
        attachments: vec![],
        timestamp: message.timestamp,
        metadata: message.metadata.clone(),
    }
}

fn can_read_recent_without_current_conversation(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(
                ActionKind::Review | ActionKind::Consolidate | ActionKind::Ruminate
            )
        )
}

async fn message_json(ctx: &SessionContext, m: &StoredMessage) -> Value {
    let ts = chrono::DateTime::from_timestamp(m.timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| m.timestamp.to_string());
    let from = if matches!(m.role, MessageRole::Assistant) {
        json!({"role": "self"})
    } else {
        let mut f = json!({"role": "user"});
        if let Some(pid) = &m.person {
            f["ref"] = json!(pid.0);
            if let Ok(Some(p)) = ctx.store.get_person(pid).await {
                if let Some(name) = &p.name {
                    f["name"] = json!(name);
                }
            }
        }
        f
    };
    let mut item = json!({
        "message_id": m.readable_message_id(),
        "time": ts,
        "from": from,
        "content": m.content,
    });
    if m.source_gateway_id.is_some() || m.source_message_id.is_some() {
        item["source"] = json!({
            "gateway_id": m.source_gateway_id.as_deref(),
            "message_id": m.source_message_id.as_deref(),
        });
    }
    if let Some(attachments) = m.metadata.get("attachments") {
        item["attachments"] = attachments.clone();
    }
    item
}

pub async fn update_conversation_summary(args: &Value, ctx: &SessionContext) -> String {
    let conv = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| current_conversation(ctx));
    let Some(conv) = conv else {
        return json!({
            "status": "error",
            "message": "No conversation specified and no current conversation.",
        })
        .to_string();
    };
    let Some(summary) = args["summary"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({
            "status": "error",
            "message": "Provide a non-empty summary.",
        })
        .to_string();
    };
    let covered_message_ids = if let Some(items) = args["covered_message_ids"].as_array() {
        items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>()
    } else {
        ctx.messages
            .iter()
            .map(|message| message.message_id.clone())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>()
    };

    match ctx
        .store
        .update_conversation_summary(&conv, summary, &covered_message_ids)
        .await
    {
        Ok(()) => json!({
            "status": "updated",
            "conversation": conv.0,
            "covered_message_ids": covered_message_ids,
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

fn parse_attachments(args: &Value) -> Result<Vec<MediaAttachment>, String> {
    if let Some(items) = args["attachments"].as_array() {
        let mut attachments = Vec::with_capacity(items.len());
        for item in items {
            if let Some(attachment) = parse_attachment(item)? {
                attachments.push(attachment);
            }
        }
        return Ok(attachments);
    }

    parse_attachment(args).map(|attachment| attachment.into_iter().collect())
}

fn parse_attachment(value: &Value) -> Result<Option<MediaAttachment>, String> {
    let Some(kind_str) = value["media_type"].as_str() else {
        return Ok(None);
    };
    let Some(kind) = MediaKind::parse(kind_str) else {
        return Err(format!("Unknown media type: {kind_str}"));
    };

    let asset_id = value["media_asset_id"]
        .as_str()
        .map(|id| MediaAssetId(id.to_string()));
    let url = if asset_id.is_some() {
        None
    } else {
        value["media_url"].as_str().map(String::from)
    };

    if asset_id.is_none() && url.is_none() {
        return Ok(None);
    }

    Ok(Some(MediaAttachment {
        kind,
        asset_id,
        url,
        mime: value["mime_type"].as_str().map(String::from),
        filename: value["filename"].as_str().map(String::from),
        size: None,
    }))
}

fn outbound_metadata(attachments: &[MediaAttachment]) -> Value {
    if attachments.is_empty() {
        Value::Null
    } else {
        serde_json::json!({ "attachments": attachments })
    }
}

async fn notify_chosen_person_of_delivery_failure(
    ctx: &SessionContext,
    target_gateway: &str,
    target_id: &str,
    conversation: Option<&ConversationId>,
    content: &str,
    error: &str,
    now: i64,
) -> bool {
    let Some(chosen_person) = chosen_person(ctx) else {
        return false;
    };
    let conversation_label = conversation
        .map(|conversation| conversation.0.as_str())
        .unwrap_or("none");
    let intent = IntentRecord {
        id: format!("intent-{}", super::util::uuid_v4()),
        kind: "scheduled".into(),
        status: "active".into(),
        task: format!(
            "Review failed outbound delivery from action {}. Target: {target_gateway}:{target_id}. Conversation: {conversation_label}. Message length: {} chars. Error: {error}. Decide whether to retry manually, update gateway setup, or ignore.",
            ctx.action_id.0,
            content.chars().count()
        ),
        person: Some(chosen_person),
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
        chosen_person_approved: true,
    };
    match ctx.store.create_intent(&intent).await {
        Ok(()) => true,
        Err(e) => {
            warn!(
                action = %ctx.action_id,
                %e,
                "failed to create chosen-person review intent for delivery failure"
            );
            false
        }
    }
}

fn chosen_person(ctx: &SessionContext) -> Option<PersonId> {
    let actor = ctx.state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, relationship)| matches!(relationship.authority, Authority::ChosenPerson))
        .map(|(person, _)| person.clone())
}

async fn wait_if_current_sender_is_typing(ctx: &SessionContext) {
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
    Some((
        msg.conversation.clone(),
        msg.gateway_id.clone(),
        msg.sender_external_id.clone(),
    ))
}

fn typing_key_is_active(ctx: &SessionContext, key: &TypingStateKey) -> bool {
    ctx.typing.read().ok().is_some_and(|typing| {
        typing
            .get(key)
            .is_some_and(|started_at| super::util::now() - started_at <= TYPING_ACTIVE_SECS)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::tools::{SessionKind, empty_delta};
    use crate::state::{ActorState, Authority, GrowthConfig};
    use crate::store::{MessageRole, SqliteStore, Store, StoredMessage};
    use async_trait::async_trait;
    use gateway::{
        GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
        GatewayRouter,
    };
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{InboundMessage, MediaAttachment, PersonId};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;

    struct RecordingAdapter {
        sent: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl GatewayAdapter for RecordingAdapter {
        async fn connect(
            _id: String,
            _db_path: String,
            _vars: BTreeMap<String, serde_json::Value>,
            _inbound_tx: mpsc::Sender<InboundMessage>,
            _gateway_event_tx: mpsc::Sender<gateway::GatewayRuntimeEvent>,
            _media_store: Arc<media::MediaStore>,
        ) -> anyhow::Result<Self>
        where
            Self: Sized,
        {
            anyhow::bail!("recording adapter is only constructed directly")
        }

        fn kind(&self) -> &str {
            "recording"
        }

        fn capabilities(&self) -> GatewayCapabilities {
            GatewayCapabilities {
                content: GatewayContentCapabilities::text_only(),
                composing: false,
                read_receipts: false,
            }
        }

        fn gateway_id(&self) -> &str {
            "relay"
        }

        fn connection_state(&self) -> GatewayConnectionState {
            GatewayConnectionState::Connected
        }

        fn setup_instructions(&self) -> Option<protocol::GatewaySetupInstructions> {
            None
        }

        async fn send_message(
            &self,
            external_id: &str,
            content: &str,
            _attachments: &[MediaAttachment],
        ) -> anyhow::Result<()> {
            self.sent
                .lock()
                .unwrap()
                .push((external_id.to_string(), content.to_string()));
            Ok(())
        }

        async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: inference::AssistantMessage {
                    text: Some(String::new()),
                    reasoning_content: None,
                    tool_calls: vec![],
                },
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("noop bridge is not used by messaging tool tests")
        }
    }

    fn test_context(
        store: Arc<SqliteStore>,
        gateway: Arc<GatewayRouter>,
        msg: InboundMessage,
    ) -> (SessionContext, mpsc::Sender<InboundMessage>) {
        let (inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let router = InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap();
        let conversation = msg.conversation.clone();

        (
            SessionContext {
                action_id: ActionId("action-test".into()),
                kind: SessionKind::Action(ActionKind::Respond),
                messages: vec![msg],
                conversation: Some(conversation),
                authority: Authority::Default,
                style_directive: None,
                cancelled_note: None,
                concurrent_summaries: vec![],
                state: StateHandle::new(shared, delta_tx),
                store,
                media_store: None,
                router: Arc::new(router),
                endpoints: vec![],
                reasoning: Reasoning::Basic,
                inject_rx,
                progress: Arc::new(RwLock::new(RunningState::new())),
                max_turns: 1,
                max_action_attempts: 1,
                escalate_after: 1,
                gateway,
                typing: Arc::new(RwLock::new(Default::default())),
                metrics: Arc::new(crate::core::ActorMetrics::default()),
                session_start: std::time::Instant::now(),
            },
            inject_tx,
        )
    }

    fn inbound() -> InboundMessage {
        InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "missing-gateway".into(),
            sender_external_id: "sender-1".into(),
            sender_display_name: Some("Sender".into()),
            reply_external_id: "reply-target".into(),
            conversation: ConversationId("missing-gateway:reply-target".into()),
            group: None,
            identity: None,
            profile: None,
            person: None,
            content: "hello".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: Value::Null,
        }
    }

    #[tokio::test]
    async fn failed_delivery_does_not_mark_response_delivered() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let gateway = Arc::new(GatewayRouter::new());
        let conv = ConversationId("missing-gateway:reply-target".into());
        let (ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(None),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = send(&json!({"content": "hi"}), &ctx, &mut state).await;

        assert!(!state.responded);
        assert!(state.attempted_send);
        assert!(result.contains("not added to visible conversation history"));
        let messages = store.get_messages(&conv, 10, None).await.unwrap();
        assert!(messages.is_empty());
        let deliveries = store
            .outbound_deliveries_for_action("action-test")
            .await
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].conversation.as_ref(), Some(&conv));
        assert_eq!(deliveries[0].gateway_id, "missing-gateway");
        assert_eq!(deliveries[0].external_id, "reply-target");
        assert_eq!(deliveries[0].status, "failed");
        assert!(deliveries[0].error.is_some());
        assert!(store.due_intents(i64::MAX, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_delivery_schedules_deduped_chosen_person_review_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let gateway = Arc::new(GatewayRouter::new());
        let chosen_person = PersonId("person-chosen_person".into());
        let (ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
        ctx.state
            .shared
            .actor
            .write()
            .unwrap()
            .set_relationship_config(&chosen_person, Some(Authority::ChosenPerson));
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(None),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = send(&json!({"content": "hi"}), &ctx, &mut state).await;
        let result_again = send(&json!({"content": "hi again"}), &ctx, &mut state).await;

        assert!(result.contains("Chosen-person review is queued"));
        assert!(result_again.contains("Chosen-person review is queued"));
        let intents = store.due_intents(i64::MAX, 10).await.unwrap();
        assert_eq!(intents.len(), 1);
        let intent = &intents[0];
        assert_eq!(intent.person.as_ref(), Some(&chosen_person));
        assert!(intent.chosen_person_approved);
        assert_eq!(intent.priority, 100);
        assert_eq!(intent.source_action.as_deref(), Some("action-test"));
        assert_eq!(
            intent.dedupe_key.as_deref(),
            Some("delivery-failure-review:action-test:missing-gateway:reply-target")
        );
        assert!(intent.task.contains("failed outbound delivery"));
        assert!(intent.task.contains("missing-gateway:reply-target"));
        assert!(intent.task.contains("Message length: 2 chars"));
    }

    #[tokio::test]
    async fn outreach_send_defaults_to_stored_conversation_reply_target() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let conv = ConversationId("relay:local".into());
        store
            .append_message(
                &conv,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "last inbound".into(),
                    identity: None,
                    profile: None,
                    person: Some(PersonId("person-sam".into())),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-1".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: Value::Null,
                },
            )
            .await
            .unwrap();

        let sent = Arc::new(Mutex::new(Vec::new()));
        let gateway = Arc::new(GatewayRouter::new());
        gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
        let (mut ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
        ctx.kind = SessionKind::Action(ActionKind::Outreach);
        ctx.messages.clear();
        ctx.conversation = Some(conv.clone());
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(None),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = send(&json!({"content": "checking in"}), &ctx, &mut state).await;

        assert_eq!(result, "Message sent.");
        assert!(state.responded);
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &[("local".to_string(), "checking in".to_string())]
        );
        assert_eq!(state.delta.relationship_changes.len(), 1);
        assert_eq!(
            state.delta.relationship_changes[0].person,
            PersonId("person-sam".into())
        );
        assert!(matches!(
            state.delta.relationship_changes[0].interaction,
            Some(RelationshipInteraction::ProactiveOutbound)
        ));
    }

    #[tokio::test]
    async fn outreach_send_marks_relationship_delta_as_proactive_outbound() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let gateway = Arc::new(GatewayRouter::new());
        gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
        let mut msg = inbound();
        msg.gateway_id = "relay".into();
        msg.reply_external_id = "local".into();
        msg.conversation = ConversationId("relay:local".into());
        msg.person = Some(protocol::PersonId("person-sam".into()));
        let (mut ctx, _inject_tx) = test_context(store, gateway, msg);
        ctx.kind = SessionKind::Action(ActionKind::Outreach);
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(None),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = send(&json!({"content": "checking in"}), &ctx, &mut state).await;

        assert_eq!(result, "Message sent.");
        assert!(state.responded);
        assert!(matches!(
            state.delta.relationship_changes[0].interaction,
            Some(RelationshipInteraction::ProactiveOutbound)
        ));
    }

    #[tokio::test]
    async fn send_waits_for_current_sender_typing_to_stop() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let gateway = Arc::new(GatewayRouter::new());
        gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
        let mut msg = inbound();
        msg.gateway_id = "relay".into();
        msg.sender_external_id = "local".into();
        msg.reply_external_id = "local".into();
        msg.conversation = ConversationId("relay:local".into());
        let key = (
            msg.conversation.clone(),
            msg.gateway_id.clone(),
            msg.sender_external_id.clone(),
        );
        let (ctx, _inject_tx) = test_context(store, gateway, msg);
        ctx.typing
            .write()
            .unwrap()
            .insert(key.clone(), crate::core::tools::util::now());
        let typing = ctx.typing.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            typing.write().unwrap().remove(&key);
        });
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(None),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let started = std::time::Instant::now();
        let result = send(&json!({"content": "hi"}), &ctx, &mut state).await;

        assert_eq!(result, "Message sent.");
        assert!(state.responded);
        assert!(started.elapsed() >= std::time::Duration::from_millis(100));
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &[("local".to_string(), "hi".to_string())]
        );
    }

    #[tokio::test]
    async fn read_messages_includes_source_message_ids_for_review_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let gateway = Arc::new(GatewayRouter::new());
        let msg = inbound();
        store
            .append_message(
                &msg.conversation,
                Some("missing-gateway"),
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "source-backed message".into(),
                    identity: None,
                    profile: None,
                    person: None,
                    source_gateway_id: Some("missing-gateway".into()),
                    source_message_id: Some("source-msg-1".into()),
                    sender_external_id: Some("sender-1".into()),
                    reply_external_id: Some("reply-target".into()),
                    metadata: Value::Null,
                },
            )
            .await
            .unwrap();
        let (ctx, _inject_tx) = test_context(store, gateway, msg);
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(None),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = read_with_state(&json!({"limit": 5}), &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["messages"][0]["message_id"], "source-msg-1");
        assert_eq!(
            parsed["messages"][0]["source"]["gateway_id"],
            "missing-gateway"
        );
        assert_eq!(
            parsed["messages"][0]["source"]["message_id"],
            "source-msg-1"
        );
        assert_eq!(state.presented_read_messages.len(), 1);
        assert_eq!(state.presented_read_messages[0].message_id, "source-msg-1");
        assert_eq!(
            state.presented_read_messages[0].gateway_id,
            "missing-gateway"
        );
        assert_eq!(
            state.presented_read_messages[0].sender_external_id,
            "sender-1"
        );

        let _ = read_with_state(&json!({"limit": 5}), &ctx, &mut state).await;
        assert_eq!(state.presented_read_messages.len(), 1);
    }

    #[tokio::test]
    async fn read_messages_assigns_local_ids_to_unsourced_messages_for_summary_coverage() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let gateway = Arc::new(GatewayRouter::new());
        let msg = inbound();
        store
            .append_message(
                &msg.conversation,
                Some("missing-gateway"),
                None,
                &StoredMessage {
                    timestamp: 1001,
                    role: MessageRole::Assistant,
                    content: "visible assistant reply".into(),
                    identity: None,
                    profile: None,
                    person: None,
                    source_gateway_id: None,
                    source_message_id: None,
                    sender_external_id: None,
                    reply_external_id: Some("reply-target".into()),
                    metadata: Value::Null,
                },
            )
            .await
            .unwrap();
        let (ctx, _inject_tx) = test_context(store.clone(), gateway, msg);

        let first_read = read(&json!({"limit": 5}), &ctx).await;
        let second_read = read(&json!({"limit": 5}), &ctx).await;
        let first: Value = serde_json::from_str(&first_read).unwrap();
        let second: Value = serde_json::from_str(&second_read).unwrap();
        let message_id = first["messages"][0]["message_id"].as_str().unwrap();

        assert!(message_id.starts_with("local:assistant:1001:"));
        assert_eq!(second["messages"][0]["message_id"], message_id);

        let result = update_conversation_summary(
            &json!({
                "summary": "Assistant replied visibly.",
                "covered_message_ids": [message_id]
            }),
            &ctx,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "updated");

        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(conversations[0].message_count, 1);
        assert_eq!(
            conversations[0].summary_covered_message_ids,
            vec![message_id.to_string()]
        );
        assert_eq!(
            conversations[0]
                .message_count
                .saturating_sub(conversations[0].summary_covered_message_ids.len() as u32),
            0
        );
    }

    #[tokio::test]
    async fn ruminate_reads_recent_messages_without_current_conversation() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let gateway = Arc::new(GatewayRouter::new());
        let newer = ConversationId("relay:newer".into());
        let older = ConversationId("relay:older".into());
        for (conversation, timestamp, content, source_message_id) in [
            (&older, 1000, "older context", "older-msg"),
            (&newer, 2000, "newer context", "newer-msg"),
        ] {
            store
                .append_message(
                    conversation,
                    Some("relay"),
                    None,
                    &StoredMessage {
                        timestamp,
                        role: MessageRole::User,
                        content: content.into(),
                        identity: None,
                        profile: None,
                        person: None,
                        source_gateway_id: Some("relay".into()),
                        source_message_id: Some(source_message_id.into()),
                        sender_external_id: Some("local".into()),
                        reply_external_id: Some("local".into()),
                        metadata: Value::Null,
                    },
                )
                .await
                .unwrap();
        }

        let (mut ctx, _inject_tx) = test_context(store, gateway, inbound());
        ctx.kind = SessionKind::Action(ActionKind::Ruminate);
        ctx.messages.clear();
        ctx.conversation = None;

        let result = read(&json!({"limit": 4}), &ctx).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let conversations = parsed["conversations"].as_array().unwrap();

        assert_eq!(conversations.len(), 2);
        assert_eq!(conversations[0]["conversation"], "relay:newer");
        assert_eq!(conversations[0]["messages"][0]["message_id"], "newer-msg");
        assert_eq!(conversations[1]["conversation"], "relay:older");
        assert_eq!(conversations[1]["messages"][0]["content"], "older context");
    }

    #[tokio::test]
    async fn default_action_without_current_conversation_cannot_read_recent_messages() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let gateway = Arc::new(GatewayRouter::new());
        let (mut ctx, _inject_tx) = test_context(store, gateway, inbound());
        ctx.messages.clear();
        ctx.conversation = None;

        let result = read(&json!({"limit": 4}), &ctx).await;

        assert_eq!(
            result,
            "No conversation specified and no current conversation."
        );
    }
}
