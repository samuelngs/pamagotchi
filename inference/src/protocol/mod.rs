mod codex_app_server;
mod openai_compatible_bridge;

pub use codex_app_server::{
    AppServerToolCall, AppServerToolResult, AppServerToolResultContent, AppServerToolRuntime,
    CodexAppServerProtocol,
};
pub use openai_compatible_bridge::OpenAiCompatibleBridge;

use std::sync::Arc;

#[derive(Clone)]
pub enum InferenceProtocol {
    OpenAiCompatible(Arc<dyn OpenAiCompatibleBridge>),
    CodexAppServer(Arc<dyn CodexAppServerProtocol>),
}

impl InferenceProtocol {
    pub async fn embed(&self, model: &str, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        match self {
            Self::OpenAiCompatible(provider) => provider.embed(model, input).await,
            Self::CodexAppServer(_) => {
                anyhow::bail!("embedding not supported by codex app-server protocol")
            }
        }
    }
}
