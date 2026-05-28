use super::stream::{parse_finish_reason, stream_sse};
use super::wire::*;
use crate::{
    AssistantMessage, ChatRequest, ChatResponse, ChatStream, Message, OpenAiCompatibleBridge,
    ToolCall, ToolChoice, Usage, message::parse_tool_arguments,
};
use anyhow::{Context, bail};
use async_trait::async_trait;
use reqwest::Client;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct OpenAiProvider {
    client: Client,
    base_url: String,
    api_key: String,
    supports_tool_choice_required: bool,
}

impl OpenAiProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            supports_tool_choice_required: true,
        }
    }

    pub fn with_tool_choice_required(mut self, supported: bool) -> Self {
        self.supports_tool_choice_required = supported;
        self
    }

    fn build_request(&self, request: &ChatRequest, stream: bool) -> WireRequest {
        let messages: Vec<WireMessage> = request
            .messages
            .iter()
            .map(|m| match m {
                Message::System(content) => WireMessage::System {
                    role: "system",
                    content: content.clone(),
                },
                Message::User(content) => WireMessage::User {
                    role: "user",
                    content: wire_user_content(content),
                },
                Message::Assistant(msg) => {
                    let tool_calls: Vec<WireToolCall> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| WireToolCall {
                            id: tc.id.clone(),
                            r#type: "function",
                            function: WireFunction {
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect();
                    WireMessage::Assistant {
                        role: "assistant",
                        content: msg.text.clone(),
                        reasoning_content: msg.reasoning_content.clone(),
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                    }
                }
                Message::Tool(result) => WireMessage::Tool {
                    role: "tool",
                    tool_call_id: result.call_id.clone(),
                    content: result.content.clone(),
                },
            })
            .collect();

        let tools: Option<Vec<WireToolDef>> = if request.tools.is_empty() {
            None
        } else {
            Some(
                request
                    .tools
                    .iter()
                    .map(|t| WireToolDef {
                        r#type: "function",
                        function: WireToolFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.parameters.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let tool_choice = if request.tools.is_empty() {
            None
        } else {
            let effective = if !self.supports_tool_choice_required
                && matches!(request.tool_choice, ToolChoice::Required)
            {
                &ToolChoice::Auto
            } else {
                &request.tool_choice
            };
            Some(match effective {
                ToolChoice::Auto => serde_json::Value::String("auto".into()),
                ToolChoice::None => serde_json::Value::String("none".into()),
                ToolChoice::Required => serde_json::Value::String("required".into()),
            })
        };

        WireRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            top_p: request.top_p,
            top_k: request.top_k,
            min_p: request.min_p,
            max_tokens: request.max_tokens,
            tools,
            tool_choice,
            response_format: request
                .response_format
                .as_ref()
                .and_then(|rf| serde_json::to_value(rf).ok()),
            stream: if stream { Some(true) } else { None },
            stream_options: if stream {
                Some(WireStreamOptions {
                    include_usage: true,
                })
            } else {
                None
            },
        }
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for OpenAiProvider {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let wire = self.build_request(request, false);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&wire)
            .send()
            .await
            .context("request failed")?;

        let status = response.status();
        let body = response.text().await.context("failed to read response")?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<WireErrorResponse>(&body) {
                bail!("API error ({}): {}", status, err.error.message);
            }
            bail!("API error ({}): {}", status, body);
        }

        let resp: WireResponse = serde_json::from_str(&body).context("failed to parse response")?;

        let choice = resp
            .choices
            .into_iter()
            .next()
            .context("no choices in response")?;

        let tool_calls: Vec<ToolCall> = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(wire_response_tool_call)
            .collect();

        let usage = resp
            .usage
            .map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            })
            .unwrap_or(Usage {
                input_tokens: 0,
                output_tokens: 0,
            });

        Ok(ChatResponse {
            message: AssistantMessage {
                text: choice.message.content,
                reasoning_content: choice.message.reasoning_content,
                tool_calls,
            },
            finish_reason: parse_finish_reason(choice.finish_reason.as_deref().unwrap_or("stop")),
            usage,
        })
    }

    async fn embed(&self, model: &str, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url);
        let body = serde_json::json!({
            "model": model,
            "input": input,
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .context("embedding request failed")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read embedding response")?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<WireErrorResponse>(&body) {
                bail!("Embedding API error ({}): {}", status, err.error.message);
            }
            bail!("Embedding API error ({}): {}", status, body);
        }

        let resp: WireEmbeddingResponse =
            serde_json::from_str(&body).context("failed to parse embedding response")?;

        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream> {
        let url = format!("{}/chat/completions", self.base_url);
        let wire = self.build_request(request, true);

        if let Ok(json) = serde_json::to_string_pretty(&wire) {
            debug!(url = %url, "LLM request:\n{json}");
        }

        info!(url = %url, model = %wire.model, "sending LLM request");
        let t0 = std::time::Instant::now();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&wire)
            .send()
            .await
            .context("request failed")?;

        let status = response.status();
        info!(status = %status, elapsed_ms = t0.elapsed().as_millis(), "LLM HTTP response received");

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if let Ok(err) = serde_json::from_str::<WireErrorResponse>(&body) {
                bail!("API error ({}): {}", status, err.error.message);
            }
            bail!("API error ({}): {}", status, body);
        }

        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            if let Err(e) = stream_sse(response, &tx).await {
                warn!(%e, "SSE stream error");
                let _ = tx.send(Err(e)).await;
            }
        });

        Ok(ChatStream::new(rx))
    }
}

fn wire_response_tool_call(tc: WireResponseToolCall) -> ToolCall {
    ToolCall {
        id: tc.id,
        name: tc.function.name,
        arguments: parse_tool_arguments(tc.function.arguments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_streaming_tool_call_preserves_malformed_arguments() {
        let call = wire_response_tool_call(WireResponseToolCall {
            id: "call-1".into(),
            function: WireResponseFunction {
                name: "send_message".into(),
                arguments: "{\"content\":".into(),
            },
        });

        assert_eq!(call.id, "call-1");
        assert_eq!(call.name, "send_message");
        assert_eq!(call.arguments["__invalid_tool_json"], true);
        assert_eq!(call.arguments["raw_arguments"], "{\"content\":");
        assert!(
            call.arguments["error"]
                .as_str()
                .is_some_and(|error| { error.contains("EOF") || error.contains("expected") })
        );
    }
}
