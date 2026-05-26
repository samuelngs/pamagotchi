use super::app_server::AppServerSession;
use super::options::CodexOptions;
use super::prompt::prompt_from_request;
use crate::{AppServerToolRuntime, ChatRequest, ChatStream, CodexAppServerProtocol};
use async_trait::async_trait;
use std::sync::Arc;

pub struct CodexProvider {
    options: CodexOptions,
}

impl CodexProvider {
    pub fn new(options: CodexOptions) -> Self {
        Self { options }
    }

    pub(super) fn app_server(&self) -> AppServerSession {
        AppServerSession::new(self.options.clone())
    }
}

#[async_trait]
impl CodexAppServerProtocol for CodexProvider {
    async fn run_turn(
        &self,
        request: &ChatRequest,
        tools: Arc<dyn AppServerToolRuntime>,
    ) -> anyhow::Result<ChatStream> {
        self.app_server()
            .chat_stream_with_tools(request.clone(), prompt_from_request(request), tools)
            .await
    }
}
