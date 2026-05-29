use super::*;
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::review::tools;
use crate::core::tools::{SessionKind, empty_delta};
use crate::identity::{Person, PersonProfileStatus, Profile};
use crate::state::{ActorState, GrowthConfig, RelationshipStanding};
use crate::store::{ActionRunRecord, MessageRole, RecallQuery, SqliteStore, Store, StoredMessage};
use async_trait::async_trait;
use gateway::GatewayRouter;
use inference::{
    Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
    InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    Usage,
};
use protocol::{ConversationId, IdentityId, InboundMessage, ProfileId};
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
        anyhow::bail!("noop bridge is not used by apply_review tests")
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
        assert_eq!(model, "embed-review");
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

fn router_with_successful_embedding_endpoint() -> inference::InferenceRouter {
    InferenceRouterBuilder::new()
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(EmbeddingBridge)),
            model: "embed-review".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Embedding],
            reasoning: Reasoning::Basic,
        })
        .build()
        .unwrap()
}

fn inbound(
    profile: &ProfileId,
    person: &PersonId,
    conversation: &ConversationId,
) -> InboundMessage {
    InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "relay".into(),
        sender: Some(protocol::ObservedSender::primary(
            "relay",
            "local",
            Some("Sam".into()),
            "test",
        )),
        channel: protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct),
        conversation: conversation.clone(),
        identity: None,
        profile: Some(profile.clone()),
        person: Some(person.clone()),
        content: "make future summaries concise".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: Value::Null,
    }
}

fn test_context(
    store: Arc<SqliteStore>,
    profile: &ProfileId,
    person: &PersonId,
    conversation: &ConversationId,
) -> (SessionContext, SessionState) {
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let router = InferenceRouterBuilder::new()
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
            model: "noop".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Chat],
            reasoning: Reasoning::Basic,
        })
        .build()
        .unwrap();
    let message = inbound(profile, person, conversation);
    let state = SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: empty_delta(Some(person.clone())),
        thoughts: vec![],
        memories_formed: vec![],
        recalled_memory_ids: vec![],
        injected_messages: vec![],
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    (
        SessionContext {
            action_id: ActionId("review-action".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages: vec![message],
            conversation: Some(conversation.clone()),
            relationship_standing: RelationshipStanding::Default,
            style_directive: None,
            cancelled_note: Some("Post-turn review for action source-action".into()),
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store,
            media_store: None,
            router: Arc::new(router),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        },
        state,
    )
}

mod evidence_tests;
mod intent_tests;
mod memory_tests;
mod person_tests;
mod structured_output_tests;
mod summary_tests;
