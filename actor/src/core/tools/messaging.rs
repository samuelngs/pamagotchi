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
mod tests;
