use crate::{ChatRequest, ChatStream};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[async_trait]
pub trait CodexAppServerProtocol: Send + Sync {
    async fn run_turn(
        &self,
        request: &ChatRequest,
        tools: Arc<dyn AppServerToolRuntime>,
    ) -> anyhow::Result<ChatStream>;
}

#[async_trait]
pub trait AppServerToolRuntime: Send + Sync {
    async fn call_tool(&self, call: AppServerToolCall) -> anyhow::Result<AppServerToolResult>;
}

#[derive(Clone, Debug)]
pub struct AppServerToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub namespace: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppServerToolResult {
    pub success: bool,
    pub content: Vec<AppServerToolResultContent>,
}

impl AppServerToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            success: true,
            content: vec![AppServerToolResultContent::Text(text.into())],
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            success: false,
            content: vec![AppServerToolResultContent::Text(text.into())],
        }
    }
}

#[derive(Clone, Debug)]
pub enum AppServerToolResultContent {
    Text(String),
    ImageUrl(String),
}
