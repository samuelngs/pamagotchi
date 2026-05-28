use super::{AssistantMessage, ChatResponse, FinishReason, ToolCall, Usage};
use crate::message::parse_tool_arguments;
use tokio::sync::mpsc;

pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallBegin {
        index: usize,
        id: String,
        name: String,
    },
    ToolCallDelta {
        index: usize,
        arguments_delta: String,
    },
    FinishReason(FinishReason),
    Usage(Usage),
}

pub struct ChatStream {
    rx: mpsc::Receiver<anyhow::Result<StreamEvent>>,
}

impl ChatStream {
    pub(crate) fn new(rx: mpsc::Receiver<anyhow::Result<StreamEvent>>) -> Self {
        Self { rx }
    }

    pub fn from_receiver(rx: mpsc::Receiver<anyhow::Result<StreamEvent>>) -> Self {
        Self { rx }
    }

    pub async fn recv(&mut self) -> Option<anyhow::Result<StreamEvent>> {
        self.rx.recv().await
    }

    pub async fn collect(mut self) -> anyhow::Result<ChatResponse> {
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut tool_calls: Vec<PartialToolCall> = vec![];
        let mut finish_reason = FinishReason::Stop;
        let mut usage = Usage {
            input_tokens: 0,
            output_tokens: 0,
        };

        while let Some(event) = self.rx.recv().await {
            match event? {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::ReasoningDelta(delta) => reasoning.push_str(&delta),
                StreamEvent::ToolCallBegin { index, id, name } => {
                    if tool_calls.len() <= index {
                        tool_calls.resize_with(index + 1, PartialToolCall::default);
                    }
                    if !id.is_empty() {
                        tool_calls[index].id = id;
                    }
                    if !name.is_empty() {
                        tool_calls[index].name = name;
                    }
                }
                StreamEvent::ToolCallDelta {
                    index,
                    arguments_delta,
                } => {
                    if tool_calls.len() <= index {
                        tool_calls.resize_with(index + 1, PartialToolCall::default);
                    }
                    tool_calls[index].arguments.push_str(&arguments_delta);
                }
                StreamEvent::FinishReason(r) => finish_reason = r,
                StreamEvent::Usage(u) => usage = u,
            }
        }

        let tool_calls = tool_calls
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.name,
                arguments: parse_tool_arguments(tc.arguments),
            })
            .collect();

        Ok(ChatResponse {
            message: AssistantMessage {
                text: if text.is_empty() { None } else { Some(text) },
                reasoning_content: if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                },
                tool_calls,
            },
            finish_reason,
            usage,
        })
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collect_preserves_malformed_tool_arguments_as_error() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(Ok(StreamEvent::ToolCallBegin {
            index: 0,
            id: "call-1".into(),
            name: "send_message".into(),
        }))
        .await
        .unwrap();
        tx.send(Ok(StreamEvent::ToolCallDelta {
            index: 0,
            arguments_delta: "{\"content\":".into(),
        }))
        .await
        .unwrap();
        drop(tx);

        let response = ChatStream::new(rx).collect().await.unwrap();
        let args = &response.message.tool_calls[0].arguments;

        assert_eq!(args["__invalid_tool_json"], true);
        assert_eq!(args["raw_arguments"], "{\"content\":");
    }
}
