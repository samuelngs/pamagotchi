use super::{recall, remember_recalled_memory};
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::{SessionContext, SessionKind, SessionState};
use crate::state::{ActorState, Delta, GrowthConfig, RelationshipStanding};
use crate::store::{Memory, MemoryKind, MemorySource, MemorySubject, SqliteStore, Store};
use async_trait::async_trait;
use gateway::GatewayRouter;
use inference::{
    Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
    InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    Usage,
};
use protocol::{ConversationId, InboundMessage, MemoryId, ProfileId};
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

struct NoopBridge;

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
        anyhow::bail!("noop bridge is not used by recall_memory tests")
    }

    async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("embedding endpoint unavailable")
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

fn state() -> SessionState {
    SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: Delta::default(),
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
    }
}

fn context(
    store: Arc<SqliteStore>,
    profile: &ProfileId,
    conversation: &ConversationId,
) -> (SessionContext, SessionState) {
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let message = InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "relay".into(),
        sender: Some(protocol::ObservedSender::primary(
            "relay", "local", None, "test",
        )),
        channel: protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct),
        conversation: conversation.clone(),
        identity: None,
        profile: Some(profile.clone()),
        person: None,
        content: "I prefer concise launch briefs.".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: Value::Null,
    };

    (
        SessionContext {
            action_id: ActionId("recall-memory-embedding-failure-test".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![message],
            conversation: Some(conversation.clone()),
            relationship_standing: RelationshipStanding::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store,
            media_store: None,
            router: Arc::new(router_with_failing_embedding_endpoint()),
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
        state(),
    )
}

#[test]
fn recalled_memory_tracking_is_deduped_and_bounded() {
    let mut state = state();
    remember_recalled_memory(&mut state, &MemoryId("memory-a".into()));
    remember_recalled_memory(&mut state, &MemoryId("memory-a".into()));
    assert_eq!(state.recalled_memory_ids, vec![MemoryId("memory-a".into())]);

    for i in 0..40 {
        remember_recalled_memory(&mut state, &MemoryId(format!("memory-{i}")));
    }

    assert_eq!(state.recalled_memory_ids.len(), 32);
    assert!(
        !state
            .recalled_memory_ids
            .contains(&MemoryId("memory-a".into()))
    );
    assert!(
        state
            .recalled_memory_ids
            .contains(&MemoryId("memory-39".into()))
    );
}

#[tokio::test]
async fn recall_uses_text_search_when_embedding_endpoint_fails() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let conversation = ConversationId("relay:local".into());
    store
        .store_memory(&Memory {
            id: MemoryId("memory-concise-launch-briefs".into()),
            kind: MemoryKind::Semantic,
            content: "Sam prefers concise launch briefs.".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            embedding: None,
            ..Memory::default()
        })
        .await
        .unwrap();
    let (ctx, mut state) = context(store, &profile, &conversation);

    let result = recall(
        &json!({
            "query": "concise launch briefs",
            "limit": 3
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let memories = parsed["memories"].as_array().unwrap();

    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["id"], "memory-concise-launch-briefs");
    assert_eq!(
        state.recalled_memory_ids,
        vec![MemoryId("memory-concise-launch-briefs".into())]
    );
}

#[tokio::test]
async fn recall_preserves_store_relevance_within_same_subject_relation() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let conversation = ConversationId("relay:local".into());

    let older_relevant = Memory {
        id: MemoryId("older-relevant".into()),
        kind: MemoryKind::Semantic,
        content: "Sam asked for a kubernetes budget review.".into(),
        source: MemorySource::Reflection,
        subjects: vec![MemorySubject::profile(
            profile.clone(),
            Some("about".into()),
            1.0,
        )],
        created_at: 1000,
        accessed_at: 1000,
        importance: 0.5,
        embedding: None,
        ..Memory::default()
    };

    let newer_weaker = Memory {
        id: MemoryId("newer-weaker".into()),
        kind: MemoryKind::Semantic,
        content: "Sam mentioned kubernetes.".into(),
        source: MemorySource::Reflection,
        subjects: vec![MemorySubject::profile(
            profile.clone(),
            Some("about".into()),
            1.0,
        )],
        created_at: 2000,
        accessed_at: 2000,
        importance: 0.5,
        embedding: None,
        ..Memory::default()
    };

    store.store_memory(&newer_weaker).await.unwrap();
    store.store_memory(&older_relevant).await.unwrap();

    let (ctx, mut state) = context(store, &profile, &conversation);
    let result = recall(
        &json!({
            "query": "kubernetes budget",
            "limit": 2
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let ids = parsed["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|memory| memory["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["older-relevant", "newer-weaker"]);
}
