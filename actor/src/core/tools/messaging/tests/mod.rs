use super::*;
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::{SessionKind, empty_delta};
use crate::state::{ActorState, GrowthConfig, RelationshipStanding};
use crate::store::{ChannelRecord, GatewayRecord, MessageRole, SqliteStore, Store, StoredMessage};
use async_trait::async_trait;
use gateway::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayRouter,
};
use inference::{
    Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
    InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    Usage,
};
use protocol::{
    ChannelId, ChannelKey, ChannelKind, GatewayId, InboundEnvelope, InboundMessage,
    MediaAttachment, ObservedSender, PersonId, channel_id,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::mpsc;

struct NoopBridge;

struct RecordingAdapter {
    sent: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait]
impl GatewayAdapter for RecordingAdapter {
    async fn connect(
        _id: String,
        _db_path: String,
        _vars: BTreeMap<String, serde_json::Value>,
        _inbound_tx: mpsc::Sender<InboundEnvelope>,
        _gateway_event_tx: mpsc::Sender<gateway::GatewayRuntimeEvent>,
        _media_store: Arc<media::MediaStore>,
    ) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        anyhow::bail!("recording adapter is only constructed directly")
    }

    fn kind(&self) -> &str {
        "recording"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities::text_only(),
            composing: false,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        "relay"
    }

    fn connection_state(&self) -> GatewayConnectionState {
        GatewayConnectionState::Connected
    }

    fn setup_instructions(&self) -> Option<protocol::GatewaySetupInstructions> {
        None
    }

    async fn send_message(
        &self,
        external_id: &str,
        content: &str,
        _attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        self.sent
            .lock()
            .unwrap()
            .push((external_id.to_string(), content.to_string()));
        Ok(())
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

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
        anyhow::bail!("noop bridge is not used by messaging tool tests")
    }
}

fn test_context(
    store: Arc<SqliteStore>,
    gateway: Arc<GatewayRouter>,
    msg: InboundMessage,
) -> (SessionContext, mpsc::Sender<InboundMessage>) {
    let (inject_tx, inject_rx) = mpsc::channel(1);
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
    let conversation = msg.conversation.clone();

    (
        SessionContext {
            action_id: ActionId("action-test".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![msg],
            conversation: Some(conversation),
            relationship_standing: RelationshipStanding::Default,
            style_directive: None,
            cancelled_note: None,
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
            gateway,
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        },
        inject_tx,
    )
}

fn inbound() -> InboundMessage {
    InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "missing-gateway".into(),
        sender: Some(ObservedSender::primary(
            "missing-gateway",
            "sender-1",
            Some("Sender".into()),
            "test",
        )),
        channel: ChannelKey::new("missing-gateway", "reply-target", ChannelKind::Direct),
        conversation: ConversationId("missing-gateway:reply-target".into()),
        identity: None,
        profile: None,
        person: None,
        content: "hello".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: Value::Null,
    }
}

async fn ensure_test_channel(
    store: &SqliteStore,
    gateway_id: &str,
    external_id: &str,
    kind: ChannelKind,
) -> ChannelId {
    let gateway = GatewayId(gateway_id.into());
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway.clone(),
            kind: gateway_id.into(),
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    let channel = ChannelRecord {
        id: channel_id(&gateway, external_id),
        gateway: gateway.clone(),
        external_id: external_id.into(),
        kind,
        space: None,
        parent: None,
        display_name: None,
        metadata: serde_json::json!({}),
        created_at: 1000,
        updated_at: 1000,
        last_seen_at: 1000,
    };
    let id = channel.id.clone();
    store.upsert_channel(&channel).await.unwrap();
    id
}

mod delivery_tests;
mod outreach_tests;
mod read_tests;
mod typing_tests;
