use super::wire::WireStreamChunk;
use crate::{FinishReason, StreamEvent, Usage};
use tokio::sync::mpsc;
use tracing::trace;

pub(super) async fn stream_sse(
    mut response: reqwest::Response,
    tx: &mpsc::Sender<anyhow::Result<StreamEvent>>,
) -> anyhow::Result<()> {
    let mut buffer = String::new();

    while let Some(chunk) = response.chunk().await? {
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim_end().to_string();
            buffer = buffer[pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            if data == "[DONE]" {
                return Ok(());
            }

            trace!(raw = %data, "SSE chunk");
            let chunk: WireStreamChunk = serde_json::from_str(data)?;

            for choice in chunk.choices {
                if let Some(content) = choice.delta.content {
                    if !content.is_empty() {
                        tx.send(Ok(StreamEvent::TextDelta(content))).await?;
                    }
                }

                if let Some(reasoning) = choice.delta.reasoning_content {
                    if !reasoning.is_empty() {
                        tx.send(Ok(StreamEvent::ReasoningDelta(reasoning))).await?;
                    }
                }

                if let Some(tool_calls) = choice.delta.tool_calls {
                    for tc in &tool_calls {
                        let name = tc.function.as_ref().and_then(|f| f.name.clone());
                        let id = tc.id.clone();

                        if id.is_some() || name.is_some() {
                            tx.send(Ok(StreamEvent::ToolCallBegin {
                                index: tc.index,
                                id: id.unwrap_or_default(),
                                name: name.unwrap_or_default(),
                            }))
                            .await?;
                        }
                        if let Some(ref func) = tc.function {
                            if let Some(ref args) = func.arguments {
                                if !args.is_empty() {
                                    tx.send(Ok(StreamEvent::ToolCallDelta {
                                        index: tc.index,
                                        arguments_delta: args.clone(),
                                    }))
                                    .await?;
                                }
                            }
                        }
                    }
                }

                if let Some(reason) = choice.finish_reason {
                    tx.send(Ok(StreamEvent::FinishReason(parse_finish_reason(&reason))))
                        .await?;
                }
            }

            if let Some(usage) = chunk.usage {
                tx.send(Ok(StreamEvent::Usage(Usage {
                    input_tokens: usage.prompt_tokens,
                    output_tokens: usage.completion_tokens,
                })))
                .await?;
            }
        }
    }

    Ok(())
}

pub(super) fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "tool_calls" => FinishReason::ToolCalls,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    }
}
