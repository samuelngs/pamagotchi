use super::{AssistantMessage, ChatResponse, FinishReason, ToolCall, Usage};
use tokio::sync::mpsc;

pub enum StreamEvent {
    TextDelta(String),
    ToolCallBegin { index: usize, id: String, name: String },
    ToolCallDelta { index: usize, arguments_delta: String },
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

    pub async fn recv(&mut self) -> Option<anyhow::Result<StreamEvent>> {
        self.rx.recv().await
    }

    pub async fn collect(mut self) -> anyhow::Result<ChatResponse> {
        let mut text = String::new();
        let mut tool_calls: Vec<PartialToolCall> = vec![];
        let mut finish_reason = FinishReason::Stop;
        let mut usage = Usage {
            input_tokens: 0,
            output_tokens: 0,
        };

        while let Some(event) = self.rx.recv().await {
            match event? {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::ToolCallBegin { index, id, name } => {
                    if tool_calls.len() <= index {
                        tool_calls.resize_with(index + 1, PartialToolCall::default);
                    }
                    tool_calls[index].id = id;
                    tool_calls[index].name = name;
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
                arguments: serde_json::from_str(&tc.arguments)
                    .unwrap_or(serde_json::Value::Object(Default::default())),
            })
            .collect();

        Ok(ChatResponse {
            message: AssistantMessage {
                text: if text.is_empty() { None } else { Some(text) },
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
