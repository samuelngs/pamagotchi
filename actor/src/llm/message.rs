#[derive(Clone)]
pub enum Message {
    System(String),
    User(String),
    Assistant(AssistantMessage),
    Tool(ToolResult),
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System(content.into())
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::User(content.into())
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant(AssistantMessage {
            text: Some(content.into()),
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
pub struct AssistantMessage {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
}
