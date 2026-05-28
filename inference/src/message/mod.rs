#[derive(Clone)]
pub enum Message {
    System(String),
    User(UserMessage),
    Assistant(AssistantMessage),
    Tool(ToolResult),
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System(content.into())
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::User(UserMessage::text(content))
    }

    pub fn user_content(parts: Vec<ContentPart>) -> Self {
        Self::User(UserMessage::Content(parts))
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant(AssistantMessage {
            text: Some(content.into()),
            reasoning_content: None,
            tool_calls: vec![],
        })
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool(ToolResult {
            call_id: call_id.into(),
            content: content.into(),
        })
    }
}

impl From<AssistantMessage> for Message {
    fn from(msg: AssistantMessage) -> Self {
        Self::Assistant(msg)
    }
}

#[derive(Clone)]
pub enum UserMessage {
    Text(String),
    Content(Vec<ContentPart>),
}

impl UserMessage {
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text(content.into())
    }

    pub fn display_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Content(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    ContentPart::ImageUrl(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    pub fn text_eq(&self, expected: &str) -> bool {
        matches!(self, Self::Text(text) if text == expected)
    }
}

#[derive(Clone)]
pub enum ContentPart {
    Text(String),
    ImageUrl(String),
}

impl ContentPart {
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text(content.into())
    }

    pub fn image_url(url: impl Into<String>) -> Self {
        Self::ImageUrl(url.into())
    }
}

#[derive(Clone)]
pub struct AssistantMessage {
    pub text: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

pub(crate) fn parse_tool_arguments(raw_arguments: impl Into<String>) -> serde_json::Value {
    let raw_arguments = raw_arguments.into();
    match serde_json::from_str(&raw_arguments) {
        Ok(arguments) => arguments,
        Err(error) => serde_json::json!({
            "__invalid_tool_json": true,
            "raw_arguments": raw_arguments,
            "error": error.to_string(),
        }),
    }
}

#[derive(Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
}

#[cfg(test)]
mod tests;
