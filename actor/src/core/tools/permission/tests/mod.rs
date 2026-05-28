use super::*;
use crate::core::action::{ActionId, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::identity::{
    Person, PersonProfileStatus, Profile, Relation, RelationSource, RelationStatus, SocialRelation,
};
use crate::state::{ActorState, GrowthConfig};
use crate::store::{
    IntentRecord, Memory, MemoryKind, MemorySource, MemorySubject, MessageRole, PrivacyCategory,
    SqliteStore, StoredMessage, VisibilityScope,
};
use gateway::GatewayRouter;
use inference::{
    Capability, InferenceEndpoint, InferenceProtocol, InferenceRouterBuilder,
    OpenAiCompatibleBridge, Reasoning, SamplingConfig,
};
use protocol::{ConversationId, IdentityId, InboundMessage, PersonId, ProfileId};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

struct NoopBridge;

#[async_trait::async_trait]
impl OpenAiCompatibleBridge for NoopBridge {
    async fn chat(
        &self,
        _request: &inference::ChatRequest,
    ) -> anyhow::Result<inference::ChatResponse> {
        anyhow::bail!("noop bridge is not used by permission tests")
    }

    async fn chat_stream(
        &self,
        _request: &inference::ChatRequest,
    ) -> anyhow::Result<inference::ChatStream> {
        anyhow::bail!("noop bridge is not used by permission tests")
    }
}

fn test_context(authority: Authority, kind: ActionKind) -> SessionContext {
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
    let message = InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "relay".into(),
        sender_external_id: "local".into(),
        sender_display_name: None,
        reply_external_id: "local".into(),
        conversation: ConversationId("relay:local".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
        content: "hello".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: serde_json::Value::Null,
    };

    SessionContext {
        action_id: ActionId("permission-test".into()),
        kind: SessionKind::Action(kind),
        messages: vec![message],
        conversation: Some(ConversationId("relay:local".into())),
        authority,
        style_directive: None,
        cancelled_note: None,
        concurrent_summaries: vec![],
        state: StateHandle::new(shared, delta_tx),
        store: Arc::new(SqliteStore::open_in_memory(4).unwrap()),
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
    }
}

async fn add_verified_target(ctx: &SessionContext, profile: &ProfileId, person: &PersonId) {
    ctx.store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Verified target".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    ctx.store
        .add_person(&Person {
            id: person.clone(),
            name: Some("Verified Person".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        })
        .await
        .unwrap();
    ctx.store
        .attach_profile_to_person(profile, person, PersonProfileStatus::Verified, 1.0, None)
        .await
        .unwrap();
}

mod identity_memory_tests;
mod intent_tests;
mod memory_lifecycle_tests;
mod memory_recall_tests;
mod messaging_tests;
mod relationship_tests;
mod review_tests;
mod social_graph_tests;
