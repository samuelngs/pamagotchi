use super::*;
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::SessionKind;
use crate::state::{ActorState, Authority, Delta, GrowthConfig};
use crate::store::{MemorySubjectType, RecallQuery, SqliteStore, Store};
use async_trait::async_trait;
use gateway::GatewayRouter;
use inference::{
    Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
    InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    Usage,
};
use protocol::{ConversationId, InboundMessage};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

struct NoopBridge;
struct EmbeddingBridge;

#[async_trait]
impl OpenAiCompatibleBridge for NoopBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            message: inference::AssistantMessage {
                text: Some(String::new()),
                reasoning_content: None,
                tool_calls: vec![],
            },
            finish_reason: FinishReason::Stop,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
            },
        })
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        anyhow::bail!("noop bridge is not used by form_memory tests")
    }

    async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("embedding endpoint unavailable")
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for EmbeddingBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("embedding bridge is not used for chat")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        anyhow::bail!("embedding bridge is not used for streaming")
    }

    async fn embed(&self, model: &str, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        assert_eq!(model, "embed-test");
        assert_eq!(input.len(), 1);
        Ok(vec![vec![0.1, 0.2, 0.3, 0.4]])
    }
}

fn router_with_failing_embedding_endpoint() -> inference::InferenceRouter {
    InferenceRouterBuilder::new()
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
            model: "chat-noop".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Chat],
            reasoning: Reasoning::Basic,
        })
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
            model: "embed-unavailable".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Embedding],
            reasoning: Reasoning::Basic,
        })
        .build()
        .unwrap()
}

mod approval_tests;
mod default_evidence_tests;
mod embedding_tests;
mod presented_evidence_tests;
mod validation_tests;
