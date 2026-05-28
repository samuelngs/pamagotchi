use super::*;

pub(super) fn prompt_hash(messages: &[Message]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for message in messages {
        match message {
            Message::System(text) => {
                "system".hash(&mut hasher);
                text.hash(&mut hasher);
            }
            Message::User(user) => {
                "user".hash(&mut hasher);
                user.display_text().hash(&mut hasher);
            }
            Message::Assistant(assistant) => {
                "assistant".hash(&mut hasher);
                assistant.text.hash(&mut hasher);
                assistant.reasoning_content.hash(&mut hasher);
                for call in &assistant.tool_calls {
                    call.id.hash(&mut hasher);
                    call.name.hash(&mut hasher);
                    call.arguments.to_string().hash(&mut hasher);
                }
            }
            Message::Tool(result) => {
                "tool".hash(&mut hasher);
                result.call_id.hash(&mut hasher);
                result.content.hash(&mut hasher);
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

pub(super) fn prompt_snapshot_messages(messages: &[Message]) -> Value {
    Value::Array(messages.iter().map(prompt_snapshot_message).collect())
}

fn prompt_snapshot_message(message: &Message) -> Value {
    match message {
        Message::System(text) => json!({
            "role": "system",
            "content": "[redacted]",
            "content_len": text.len(),
        }),
        Message::User(user) => prompt_snapshot_user_message(user),
        Message::Assistant(assistant) => {
            let tool_calls = assistant
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id.as_str(),
                        "name": call.name.as_str(),
                        "arguments": redact_prompt_trace_value(&call.arguments),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "role": "assistant",
                "content": assistant.text.as_ref().map(|_| "[redacted]"),
                "content_len": assistant.text.as_ref().map(|text| text.len()).unwrap_or(0),
                "reasoning_len": assistant
                    .reasoning_content
                    .as_deref()
                    .map(str::len)
                    .unwrap_or(0),
                "tool_calls": tool_calls,
            })
        }
        Message::Tool(result) => json!({
            "role": "tool",
            "call_id": result.call_id.as_str(),
            "content": prompt_snapshot_tool_content(&result.content),
        }),
    }
}

fn prompt_snapshot_user_message(user: &UserMessage) -> Value {
    match user {
        UserMessage::Text(text) => json!({
            "role": "user",
            "content": "[redacted]",
            "content_len": text.len(),
        }),
        UserMessage::Content(parts) => {
            let content_parts = parts
                .iter()
                .map(prompt_snapshot_content_part)
                .collect::<Vec<_>>();
            json!({
                "role": "user",
                "content": "[redacted]",
                "content_len": user.display_text().len(),
                "content_parts": content_parts,
            })
        }
    }
}

fn prompt_snapshot_content_part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text(text) => json!({
            "type": "text",
            "content": "[redacted]",
            "content_len": text.len(),
        }),
        ContentPart::ImageUrl(url) => json!({
            "type": "image_url",
            "url": redact_prompt_image_url(url),
        }),
    }
}

fn prompt_snapshot_tool_content(content: &str) -> Value {
    match serde_json::from_str::<Value>(content) {
        Ok(parsed) if parsed.is_object() || parsed.is_array() => redact_prompt_trace_value(&parsed),
        _ => Value::String(content.to_string()),
    }
}

fn redact_prompt_image_url(url: &str) -> &'static str {
    if url.starts_with("data:") {
        "[inline image redacted]"
    } else {
        "[image url redacted]"
    }
}

fn redact_prompt_trace_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if should_redact_prompt_trace_key(key) {
                        Value::String("[redacted]".into())
                    } else {
                        redact_prompt_trace_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_prompt_trace_value).collect()),
        Value::String(text) => {
            let trimmed = text.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                match serde_json::from_str::<Value>(text) {
                    Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                        Value::String(redact_prompt_trace_value(&parsed).to_string())
                    }
                    _ => value.clone(),
                }
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn should_redact_prompt_trace_key(key: &str) -> bool {
    matches!(
        key,
        "content"
            | "text"
            | "summary"
            | "comm_style"
            | "evidence_quote"
            | "reason"
            | "task"
            | "external_id"
            | "sender_external_id"
            | "reply_external_id"
            | "source_message_id"
            | "media_url"
            | "url"
            | "raw_arguments"
    )
}
