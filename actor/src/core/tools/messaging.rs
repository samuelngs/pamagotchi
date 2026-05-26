use super::context::{SessionContext, SessionState};
use crate::store::{MessageRole, StoredMessage};
use inference::Tool;
use protocol::{ConversationId, MediaAttachment, MediaKind};
use serde_json::{Value, json};
use tracing::warn;

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
                        "description": "URL of media to attach"
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
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "read_messages".into(),
            description: "Read messages from a conversation. Use to access older history beyond what's in your current context.".into(),
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
    ]
}

fn current_conversation(ctx: &SessionContext) -> Option<ConversationId> {
    ctx.conversation
        .clone()
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()))
}

pub async fn send(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let content = args["content"].as_str().unwrap_or("").to_string();
    let gateway_id = args["gateway_id"].as_str();
    let external_id = args["external_id"].as_str();

    let media = match (args["media_url"].as_str(), args["media_type"].as_str()) {
        (Some(url), Some(kind_str)) => match MediaKind::parse(kind_str) {
            Some(kind) => Some(MediaAttachment {
                kind,
                url: Some(url.to_string()),
                mime: args["mime_type"].as_str().map(String::from),
                filename: args["filename"].as_str().map(String::from),
                size: None,
            }),
            None => return format!("Unknown media type: {kind_str}"),
        },
        _ => None,
    };

    let is_outbound = gateway_id.is_some() && external_id.is_some();

    let (target_gateway, target_id) = if is_outbound {
        (
            gateway_id.unwrap().to_string(),
            external_id.unwrap().to_string(),
        )
    } else if let Some(msg) = ctx.messages.first() {
        (msg.gateway_id.clone(), msg.external_id.clone())
    } else {
        state.responded = true;
        return "No delivery target — message not sent.".into();
    };

    let delivery = ctx
        .gateway
        .send_message(&target_gateway, &target_id, &content, media.as_ref())
        .await;

    if !state.composing_released {
        ctx.gateway
            .release_composing(&target_gateway, &target_id)
            .await;
        state.composing_released = true;
    }

    if let Some(conv) = current_conversation(ctx) {
        let stored = StoredMessage {
            timestamp: super::util::now(),
            role: MessageRole::Assistant,
            content: content.clone(),
            identity: None,
            profile: None,
            person: None,
            metadata: Value::Null,
        };
        ctx.store
            .append_message(&conv, Some(&target_gateway), None, &stored)
            .await
            .ok();
    }

    state.responded = true;

    match delivery {
        Ok(_) => {
            if is_outbound {
                format!("Message sent to {target_gateway}:{target_id}.")
            } else {
                "Message sent.".into()
            }
        }
        Err(e) => {
            warn!(
                action = %ctx.action_id,
                %e,
                gateway = %target_gateway,
                "message delivery failed"
            );
            format!("Message stored but delivery failed: {e}")
        }
    }
}

pub async fn read(args: &Value, ctx: &SessionContext) -> String {
    let conv = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| current_conversation(ctx));

    let Some(conv) = conv else {
        return "No conversation specified and no current conversation.".into();
    };

    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let before = args["before"].as_i64();

    match ctx.store.get_messages(&conv, limit, before).await {
        Ok(messages) if messages.is_empty() => json!({"messages": []}).to_string(),
        Ok(messages) => {
            let mut items = Vec::new();
            for m in &messages {
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
                items.push(json!({
                    "time": ts,
                    "from": from,
                    "content": m.content,
                }));
            }
            json!({"messages": items}).to_string()
        }
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}
