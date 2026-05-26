use crate::{ChatRequest, ChatResponse, ChatStream};
use async_trait::async_trait;

#[async_trait]
pub trait OpenAiCompatibleBridge: Send + Sync {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse>;
    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream>;

    async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("embedding not supported by this provider")
    }
}
