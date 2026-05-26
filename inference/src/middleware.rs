use super::{
    AppServerToolRuntime, ChatRequest, ChatResponse, ChatStream, CodexAppServerProtocol,
    OpenAiCompatibleBridge,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Retry<P> {
    inner: P,
    max_attempts: u32,
    base_delay: Duration,
}

impl<P> Retry<P> {
    pub fn new(inner: P, max_attempts: u32) -> Self {
        Self {
            inner,
            max_attempts,
            base_delay: Duration::from_secs(1),
        }
    }

    pub fn with_base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = delay;
        self
    }
}

#[async_trait]
impl<P: OpenAiCompatibleBridge> OpenAiCompatibleBridge for Retry<P> {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        let mut last_err = None;
        for attempt in 0..self.max_attempts {
            match self.inner.chat(request).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max = self.max_attempts,
                        error = %e,
                        "retrying chat"
                    );
                    last_err = Some(e);
                    if attempt + 1 < self.max_attempts {
                        tokio::time::sleep(self.base_delay * 2u32.pow(attempt)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }

    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream> {
        let mut last_err = None;
        for attempt in 0..self.max_attempts {
            match self.inner.chat_stream(request).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max = self.max_attempts,
                        error = %e,
                        "retrying chat stream"
                    );
                    last_err = Some(e);
                    if attempt + 1 < self.max_attempts {
                        tokio::time::sleep(self.base_delay * 2u32.pow(attempt)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }
}

#[async_trait]
impl<P: CodexAppServerProtocol> CodexAppServerProtocol for Retry<P> {
    async fn run_turn(
        &self,
        request: &ChatRequest,
        tools: Arc<dyn AppServerToolRuntime>,
    ) -> anyhow::Result<ChatStream> {
        let mut last_err = None;
        for attempt in 0..self.max_attempts {
            match self.inner.run_turn(request, tools.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max = self.max_attempts,
                        error = %e,
                        "retrying codex app-server turn"
                    );
                    last_err = Some(e);
                    if attempt + 1 < self.max_attempts {
                        tokio::time::sleep(self.base_delay * 2u32.pow(attempt)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }
}

pub struct Logging<P> {
    inner: P,
}

impl<P> Logging<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<P: OpenAiCompatibleBridge> OpenAiCompatibleBridge for Logging<P> {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        tracing::debug!(
            model = %request.model,
            messages = request.messages.len(),
            tools = request.tools.len(),
            "chat request"
        );

        let start = Instant::now();
        let result = self.inner.chat(request).await;
        let elapsed = start.elapsed();

        match &result {
            Ok(response) => tracing::debug!(
                input_tokens = response.usage.input_tokens,
                output_tokens = response.usage.output_tokens,
                tool_calls = response.message.tool_calls.len(),
                elapsed_ms = elapsed.as_millis() as u64,
                "chat response"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                elapsed_ms = elapsed.as_millis() as u64,
                "chat error"
            ),
        }

        result
    }

    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream> {
        tracing::debug!(
            model = %request.model,
            messages = request.messages.len(),
            tools = request.tools.len(),
            "chat stream request"
        );

        let start = Instant::now();
        let result = self.inner.chat_stream(request).await;

        match &result {
            Ok(_) => tracing::debug!(
                elapsed_ms = start.elapsed().as_millis() as u64,
                "chat stream connected"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "chat stream error"
            ),
        }

        result
    }
}

pub struct Timeout<P> {
    inner: P,
    duration: Duration,
}

impl<P> Timeout<P> {
    pub fn new(inner: P, duration: Duration) -> Self {
        Self { inner, duration }
    }
}

#[async_trait]
impl<P: OpenAiCompatibleBridge> OpenAiCompatibleBridge for Timeout<P> {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        tokio::time::timeout(self.duration, self.inner.chat(request))
            .await
            .map_err(|_| anyhow::anyhow!("chat timed out after {:?}", self.duration))?
    }

    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream> {
        tokio::time::timeout(self.duration, self.inner.chat_stream(request))
            .await
            .map_err(|_| anyhow::anyhow!("chat stream timed out after {:?}", self.duration))?
    }
}
