use crate::{ChatRequest, ContentPart, Message, Tool, ToolResult, UserMessage};

pub(super) fn prompt_from_request(request: &ChatRequest) -> String {
    let mut out = String::new();

    for message in &request.messages {
        match message {
            Message::System(content) => push_section(&mut out, "System", content),
            Message::User(content) => push_section(&mut out, "User", &user_text(content)),
            Message::Assistant(message) => {
                if let Some(text) = &message.text {
                    push_section(&mut out, "Assistant", text);
                }
                if !message.tool_calls.is_empty() {
                    let calls = message
                        .tool_calls
                        .iter()
                        .map(|call| {
                            format!(
                                "- {} ({}) {}",
                                call.name,
                                call.id,
                                serde_json::to_string(&call.arguments)
                                    .unwrap_or_else(|_| "{}".into())
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    push_section(&mut out, "Assistant tool calls", &calls);
                }
            }
            Message::Tool(result) => push_section(&mut out, "Tool result", &tool_result(result)),
        }
    }

    if !request.tools.is_empty() {
        push_section(
            &mut out,
            "Available application tools",
            &tools_text(&request.tools),
        );
        out.push_str(
            "\nThe application tool list above is contextual. Do not claim that you executed those tools unless their results are present in the transcript.\n",
        );
    }

    out
}

fn push_section(out: &mut String, label: &str, content: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str("## ");
    out.push_str(label);
    out.push('\n');
    out.push_str(content.trim());
    out.push('\n');
}

fn user_text(content: &UserMessage) -> String {
    match content {
        UserMessage::Text(text) => text.clone(),
        UserMessage::Content(parts) => parts
            .iter()
            .map(|part| match part {
                ContentPart::Text(text) => text.clone(),
                ContentPart::ImageUrl(url) => format!("[image: {url}]"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn tool_result(result: &ToolResult) -> String {
    format!("call_id: {}\n{}", result.call_id, result.content)
}

fn tools_text(tools: &[Tool]) -> String {
    tools
        .iter()
        .map(|tool| {
            format!(
                "- {}: {}\n  parameters: {}",
                tool.name,
                tool.description,
                serde_json::to_string(&tool.parameters).unwrap_or_else(|_| "{}".into())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
