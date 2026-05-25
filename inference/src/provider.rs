use super::{ChatRequest, ChatResponse, ChatStream};
use async_trait::async_trait;

#[async_trait]
pub trait Provider: Send + Sync {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse>;
    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream>;
}
