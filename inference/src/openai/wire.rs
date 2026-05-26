use crate::{ContentPart, UserMessage};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(super) struct WireRequest {
    pub(super) model: String,
    pub(super) messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<WireToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream_options: Option<WireStreamOptions>,
}

#[derive(Serialize)]
pub(super) struct WireStreamOptions {
    pub(super) include_usage: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum WireMessage {
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

pub(super) fn wire_user_content(content: &UserMessage) -> WireUserContent {
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
pub(super) enum WireUserContent {
    Text(String),
    Parts(Vec<WireContentPart>),
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum WireContentPart {
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
pub(super) struct WireImageUrl {
    url: String,
}

#[derive(Serialize)]
pub(super) struct WireToolCall {
    pub(super) id: String,
    pub(super) r#type: &'static str,
    pub(super) function: WireFunction,
}

#[derive(Serialize)]
pub(super) struct WireFunction {
    pub(super) name: String,
    pub(super) arguments: String,
}

#[derive(Serialize)]
pub(super) struct WireToolDef {
    pub(super) r#type: &'static str,
    pub(super) function: WireToolFunction,
}

#[derive(Serialize)]
pub(super) struct WireToolFunction {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) parameters: serde_json::Value,
}

#[derive(Deserialize)]
pub(super) struct WireResponse {
    pub(super) choices: Vec<WireChoice>,
    pub(super) usage: Option<WireUsage>,
}

#[derive(Deserialize)]
pub(super) struct WireChoice {
    pub(super) message: WireResponseMessage,
    pub(super) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct WireResponseMessage {
    pub(super) content: Option<String>,
    pub(super) reasoning_content: Option<String>,
    pub(super) tool_calls: Option<Vec<WireResponseToolCall>>,
}

#[derive(Deserialize)]
pub(super) struct WireResponseToolCall {
    pub(super) id: String,
    pub(super) function: WireResponseFunction,
}

#[derive(Deserialize)]
pub(super) struct WireResponseFunction {
    pub(super) name: String,
    pub(super) arguments: String,
}

#[derive(Deserialize)]
pub(super) struct WireUsage {
    pub(super) prompt_tokens: u32,
    pub(super) completion_tokens: u32,
}

#[derive(Deserialize)]
pub(super) struct WireErrorResponse {
    pub(super) error: WireErrorDetail,
}

#[derive(Deserialize)]
pub(super) struct WireErrorDetail {
    pub(super) message: String,
}

#[derive(Deserialize)]
pub(super) struct WireStreamChunk {
    pub(super) choices: Vec<WireStreamChoice>,
    pub(super) usage: Option<WireUsage>,
}

#[derive(Deserialize)]
pub(super) struct WireStreamChoice {
    pub(super) delta: WireStreamDelta,
    pub(super) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct WireStreamDelta {
    pub(super) content: Option<String>,
    pub(super) reasoning_content: Option<String>,
    pub(super) tool_calls: Option<Vec<WireStreamToolCallDelta>>,
}

#[derive(Deserialize)]
pub(super) struct WireStreamToolCallDelta {
    pub(super) index: usize,
    pub(super) id: Option<String>,
    pub(super) function: Option<WireStreamFunctionDelta>,
}

#[derive(Deserialize)]
pub(super) struct WireStreamFunctionDelta {
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct WireEmbeddingResponse {
    pub(super) data: Vec<WireEmbeddingData>,
}

#[derive(Deserialize)]
pub(super) struct WireEmbeddingData {
    pub(super) embedding: Vec<f32>,
}
