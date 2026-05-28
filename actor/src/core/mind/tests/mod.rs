use super::*;
use crate::core::Outcome;
use crate::core::decision::{MindDecision, MindVerdict};
use crate::core::event::FiredIntent;
use crate::core::handle::{SharedState, StateTask};
use crate::identity::{Identity, Profile};
use crate::state::{ActorState, Authority, GrowthConfig, ProactiveConsent, QuietHoursUtc};
use crate::store::{
    EventInboxRecord, IntentRecord, Memory, MemoryKind, MemorySource, MemorySubject, MessageRole,
    RecallQuery, SqliteStore, StoredMessage, Thought, ThoughtKind,
};
use async_trait::async_trait;
use gateway::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
};
use inference::{
    AssistantMessage, Capability, ChatRequest, ChatResponse, ChatStream, FinishReason,
    InferenceEndpoint, InferenceProtocol, InferenceRouter, InferenceRouterBuilder,
    OpenAiCompatibleBridge, Reasoning, SamplingConfig, Usage,
};
use protocol::{
    ConversationId, GroupId, IdentityId, InboundMessage, MediaAttachment, MemoryId, PersonId,
    ProfileId,
};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

struct NoopBridge;
struct StateAdapter {
    state: GatewayConnectionState,
}

#[async_trait]
impl OpenAiCompatibleBridge for NoopBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            message: AssistantMessage {
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
        anyhow::bail!("noop bridge is not used by identity resolution tests")
    }
}

fn test_mind(store: Arc<dyn Store>) -> Mind {
    test_mind_with_gateway_state(store, GatewayConnectionState::Connected)
}

fn test_mind_with_gateway_state(
    store: Arc<dyn Store>,
    gateway_state: GatewayConnectionState,
) -> Mind {
    let (mind, _) = test_mind_with_gateway_state_and_event_receiver(store, gateway_state);
    mind
}

fn test_router() -> Arc<InferenceRouter> {
    Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    )
}

fn test_mind_with_gateway_state_and_event_receiver(
    store: Arc<dyn Store>,
    gateway_state: GatewayConnectionState,
) -> (Mind, mpsc::Receiver<WakeEvent>) {
    let (_event_tx, event_rx) = mpsc::channel(4);
    let (event_tx, external_rx) = mpsc::channel(4);
    let (delta_tx, _delta_rx) = mpsc::channel(4);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state = StateHandle::new(shared, delta_tx);
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(StateAdapter {
        state: gateway_state,
    }));

    (
        Mind::new(
            event_rx,
            event_tx,
            state,
            store,
            None,
            test_router(),
            gateway,
            5,
            5,
            1,
            1,
            Arc::new(ActorMetrics::default()),
            None,
        ),
        external_rx,
    )
}

fn test_mind_with_state_task(store: Arc<SqliteStore>) -> (Mind, tokio::task::JoinHandle<()>) {
    let (_event_tx, event_rx) = mpsc::channel(4);
    let (event_tx, _external_rx) = mpsc::channel(4);
    let (state_tx, state_rx) = mpsc::channel(4);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state_task = StateTask::new(shared.clone(), store.clone(), state_rx, None);
    let state_join = tokio::spawn(async move {
        state_task.run().await;
    });
    let state = StateHandle::new(shared, state_tx);
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(StateAdapter {
        state: GatewayConnectionState::Connected,
    }));

    (
        Mind::new(
            event_rx,
            event_tx,
            state,
            store,
            None,
            test_router(),
            gateway,
            5,
            5,
            1,
            1,
            Arc::new(ActorMetrics::default()),
            None,
        ),
        state_join,
    )
}

#[async_trait]
impl GatewayAdapter for StateAdapter {
    async fn connect(
        _id: String,
        _db_path: String,
        _vars: BTreeMap<String, serde_json::Value>,
        _inbound_tx: mpsc::Sender<InboundMessage>,
        _gateway_event_tx: mpsc::Sender<gateway::GatewayRuntimeEvent>,
        _media_store: Arc<media::MediaStore>,
    ) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        anyhow::bail!("state adapter is only constructed directly")
    }

    fn kind(&self) -> &str {
        "state"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities::text_only(),
            composing: true,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        "relay"
    }

    fn connection_state(&self) -> GatewayConnectionState {
        self.state.clone()
    }

    fn setup_instructions(&self) -> Option<protocol::GatewaySetupInstructions> {
        None
    }

    async fn send_message(
        &self,
        _external_id: &str,
        _content: &str,
        _attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn inbound(
    gateway_id: &str,
    sender: &str,
    display_name: &str,
    reply_target: &str,
    conversation: &str,
    group: Option<&str>,
    message_id: &str,
) -> InboundMessage {
    InboundMessage {
        message_id: message_id.into(),
        gateway_id: gateway_id.into(),
        sender_external_id: sender.into(),
        sender_display_name: Some(display_name.into()),
        reply_external_id: reply_target.into(),
        conversation: ConversationId(conversation.into()),
        group: group.map(|id| GroupId(id.into())),
        identity: None,
        profile: None,
        person: None,
        content: "hello".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: serde_json::Value::Null,
    }
}

async fn append_inbound(store: &dyn Store, msg: &InboundMessage) {
    let stored = StoredMessage {
        timestamp: msg.timestamp,
        role: MessageRole::User,
        content: msg.content.clone(),
        identity: msg.identity.clone(),
        profile: msg.profile.clone(),
        person: msg.person.clone(),
        source_gateway_id: Some(msg.gateway_id.clone()),
        source_message_id: Some(msg.message_id.clone()),
        sender_external_id: Some(msg.sender_external_id.clone()),
        reply_external_id: Some(msg.reply_external_id.clone()),
        metadata: serde_json::Value::Null,
    };
    store
        .append_message(
            &msg.conversation,
            Some(&msg.gateway_id),
            msg.group.as_ref(),
            &stored,
        )
        .await
        .unwrap();
}

fn recent_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn set_proactive_consent(mind: &Mind, person: &PersonId, consent: ProactiveConsent) {
    let mut actor = mind.state.shared.actor.write().unwrap();
    actor
        .bonds
        .entry(person.clone())
        .or_default()
        .proactive_consent = consent;
}

fn allow_proactive(mind: &Mind, person: &PersonId) {
    set_proactive_consent(mind, person, ProactiveConsent::Allowed);
}

fn set_unanswered_proactive_outreach(mind: &Mind, person: &PersonId) {
    let now = recent_timestamp();
    let mut actor = mind.state.shared.actor.write().unwrap();
    let rel = actor.bonds.entry(person.clone()).or_default();
    rel.last_inbound = now - 60;
    rel.last_proactive_outbound = now - 30;
    rel.proactive_outbound_count = 1;
}

fn fill_capacity_with_running_responses(mind: &mut Mind) {
    for idx in 0..5 {
        let conversation = format!("relay:running-{idx}");
        let msg = inbound(
            "relay",
            &format!("running-{idx}"),
            "Sam",
            &format!("running-{idx}"),
            &conversation,
            None,
            &format!("running-msg-{idx}"),
        );
        let action = Action::respond(
            vec![msg.clone()],
            msg.conversation.clone(),
            Authority::Default,
            None,
        );
        let id = mind.registry.schedule(action);
        mind.registry.launch(&id).expect("action launches");
    }
}

mod approval_tests;
mod consolidation_tests;
mod deferral_tests;
mod event_recovery_tests;
mod intent_completion_tests;
mod lifecycle_tests;
mod proactive_tests;
mod profile_ingest_tests;
mod review_trigger_tests;
mod typing_tests;
