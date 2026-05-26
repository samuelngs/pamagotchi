use super::{
    AssistantMessage, ChatRequest, ChatResponse, ChatStream, ContentPart, FinishReason, Message,
    Provider, StreamEvent, ToolCall, ToolChoice, Usage, UserMessage,
};
use anyhow::{Context, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

#[derive(Clone, Serialize, Deserialize)]
pub struct OpenAiOptions {
    pub model: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,

    #[serde(default = "default_true")]
    pub tool_choice_required: bool,
}

fn default_true() -> bool {
    true
}

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
impl Provider for OpenAiProvider {
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
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments: serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::Object(Default::default())),
            })
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

async fn stream_sse(
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

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "tool_calls" => FinishReason::ToolCalls,
        "length" => FinishReason::Length,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    }
}

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<WireStreamOptions>,
}

#[derive(Serialize)]
struct WireStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireMessage {
    System {
        role: &'static str,
        content: String,
    },
    User {
        role: &'static str,
        content: WireUserContent,
    },
    Assistant {
        role: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<WireToolCall>>,
    },
    Tool {
        role: &'static str,
        tool_call_id: String,
        content: String,
    },
}

fn wire_user_content(content: &UserMessage) -> WireUserContent {
    match content {
        UserMessage::Text(text) => WireUserContent::Text(text.clone()),
        UserMessage::Content(parts) => WireUserContent::Parts(
            parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => WireContentPart::Text {
                        r#type: "text",
                        text: text.clone(),
                    },
                    ContentPart::ImageUrl(url) => WireContentPart::ImageUrl {
                        r#type: "image_url",
                        image_url: WireImageUrl { url: url.clone() },
                    },
                })
                .collect(),
        ),
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireUserContent {
    Text(String),
    Parts(Vec<WireContentPart>),
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireContentPart {
    Text {
        r#type: &'static str,
        text: String,
    },
    ImageUrl {
        r#type: &'static str,
        image_url: WireImageUrl,
    },
}

#[derive(Serialize)]
struct WireImageUrl {
    url: String,
}

#[derive(Serialize)]
struct WireToolCall {
    id: String,
    r#type: &'static str,
    function: WireFunction,
}

#[derive(Serialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct WireToolDef {
    r#type: &'static str,
    function: WireToolFunction,
}

#[derive(Serialize)]
struct WireToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireResponseMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<WireResponseToolCall>>,
}

#[derive(Deserialize)]
struct WireResponseToolCall {
    id: String,
    function: WireResponseFunction,
}

#[derive(Deserialize)]
struct WireResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct WireErrorResponse {
    error: WireErrorDetail,
}

#[derive(Deserialize)]
struct WireErrorDetail {
    message: String,
}

#[derive(Deserialize)]
struct WireStreamChunk {
    choices: Vec<WireStreamChoice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireStreamChoice {
    delta: WireStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireStreamDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<WireStreamToolCallDelta>>,
}

#[derive(Deserialize)]
struct WireStreamToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<WireStreamFunctionDelta>,
}

#[derive(Deserialize)]
struct WireStreamFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct WireEmbeddingResponse {
    data: Vec<WireEmbeddingData>,
}

#[derive(Deserialize)]
struct WireEmbeddingData {
    embedding: Vec<f32>,
}
