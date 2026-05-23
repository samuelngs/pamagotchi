use super::{AssistantMessage, ToolCall};

pub struct ChatResponse {
    pub message: AssistantMessage,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}

impl ChatResponse {
    pub fn text(&self) -> Option<&str> {
        self.message.text.as_deref()
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.message.tool_calls
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.message.tool_calls.is_empty()
    }
}

pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
}

pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}
