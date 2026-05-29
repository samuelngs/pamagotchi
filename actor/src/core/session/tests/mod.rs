use super::messages::{
    inject_pending_messages, message_metadata, remember_injected_message, required_capabilities,
    source_message_keys,
};
use super::{SessionResult, build_result, prompt_snapshot_messages, run_session};
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::{SessionContext, SessionKind, SessionState, empty_delta};
use crate::state::{ActorState, GrowthConfig, RelationshipStanding};
use crate::store::{MessageRole, SqliteStore, Store, Thought, ThoughtKind};
use async_trait::async_trait;
use gateway::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayRouter,
};
use inference::{
    AssistantMessage, Capability, ChatRequest, ChatResponse, ChatStream, ContentPart, FinishReason,
    InferenceEndpoint, InferenceProtocol, InferenceRouterBuilder, Message, OpenAiCompatibleBridge,
    Reasoning, RouteContext, SamplingConfig, ToolCall, Usage, UserMessage,
};
use protocol::{
    ConversationId, InboundMessage, MediaAssetId, MediaAttachment, MediaKind, MemoryId,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::mpsc;

fn inbound(metadata: Value) -> InboundMessage {
    InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "whatsapp".into(),
        sender_external_id: "sender-1".into(),
        sender_display_name: Some("Sender".into()),
        reply_external_id: "chat-1".into(),
        conversation: ConversationId("whatsapp:chat-1".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
        content: String::new(),
        attachments: vec![MediaAttachment {
            kind: MediaKind::Sticker,
            asset_id: Some(MediaAssetId("media-1".into())),
            url: None,
            mime: Some("image/webp".into()),
            filename: Some("sticker.webp".into()),
            size: Some(99),
        }],
        timestamp: 1,
        metadata,
    }
}

fn text_inbound(message_id: &str, content: &str) -> InboundMessage {
    InboundMessage {
        message_id: message_id.into(),
        gateway_id: "whatsapp".into(),
        sender_external_id: "sender-1".into(),
        sender_display_name: Some("Sender".into()),
        reply_external_id: "chat-1".into(),
        conversation: ConversationId("whatsapp:chat-1".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
        content: content.into(),
        attachments: vec![],
        timestamp: 1,
        metadata: Value::Null,
    }
}

struct NoopBridge;

struct HangingBridge;

struct TextThenSendBridge {
    calls: AtomicUsize,
}

struct SendOnlyBridge {
    calls: AtomicUsize,
}

struct RecordingAdapter {
    sent: Arc<Mutex<Vec<String>>>,
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
        anyhow::bail!("noop bridge is not used by session message tests")
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for HangingBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("hanging bridge is only used for streaming")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        std::future::pending::<anyhow::Result<ChatStream>>().await
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for TextThenSendBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("scripted bridge is only used for streaming")
    }

    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(4);
        if call == 0 {
            tx.send(Ok(inference::StreamEvent::TextDelta(
                "This text is not visible to the user.".into(),
            )))
            .await
            .unwrap();
            tx.send(Ok(inference::StreamEvent::FinishReason(FinishReason::Stop)))
                .await
                .unwrap();
        } else {
            assert!(
                request
                    .messages
                    .iter()
                    .any(|message| matches!(message, Message::System(text) if text.contains("previous attempt failed to call send_message"))),
                "retry prompt should warn that text without a tool call is silent"
            );
            tx.send(Ok(inference::StreamEvent::ToolCallBegin {
                index: 0,
                id: "call-send".into(),
                name: "send_message".into(),
            }))
            .await
            .unwrap();
            tx.send(Ok(inference::StreamEvent::ToolCallDelta {
                index: 0,
                arguments_delta: r#"{"content":"visible reply"}"#.into(),
            }))
            .await
            .unwrap();
            tx.send(Ok(inference::StreamEvent::FinishReason(
                FinishReason::ToolCalls,
            )))
            .await
            .unwrap();
        }
        drop(tx);
        Ok(ChatStream::from_receiver(rx))
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for SendOnlyBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("scripted bridge is only used for streaming")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(4);
        tx.send(Ok(inference::StreamEvent::ToolCallBegin {
            index: 0,
            id: "call-send".into(),
            name: "send_message".into(),
        }))
        .await
        .unwrap();
        tx.send(Ok(inference::StreamEvent::ToolCallDelta {
            index: 0,
            arguments_delta: r#"{"content":"visible reply"}"#.into(),
        }))
        .await
        .unwrap();
        tx.send(Ok(inference::StreamEvent::FinishReason(
            FinishReason::ToolCalls,
        )))
        .await
        .unwrap();
        drop(tx);
        Ok(ChatStream::from_receiver(rx))
    }
}

#[async_trait]
impl GatewayAdapter for RecordingAdapter {
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
        "whatsapp"
    }

    fn connection_state(&self) -> GatewayConnectionState {
        GatewayConnectionState::Connected
    }

    fn setup_instructions(&self) -> Option<protocol::GatewaySetupInstructions> {
        None
    }

    async fn send_message(
        &self,
        _external_id: &str,
        content: &str,
        _attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(content.to_string());
        Ok(())
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn test_context(store: Arc<SqliteStore>, source: InboundMessage) -> SessionContext {
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
    let conversation = source.conversation.clone();

    SessionContext {
        action_id: ActionId("action-test".into()),
        kind: SessionKind::Action(ActionKind::Respond),
        messages: vec![source],
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
        gateway: Arc::new(GatewayRouter::new()),
        typing: Arc::new(RwLock::new(Default::default())),
        metrics: Arc::new(crate::core::ActorMetrics::default()),
        session_start: std::time::Instant::now(),
    }
}

async fn wait_for_composing_count(
    gateway: &GatewayRouter,
    gateway_id: &str,
    external_id: &str,
    expected: usize,
) {
    for _ in 0..20 {
        if gateway.composing_count(gateway_id, external_id).await == expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

mod composing_tests;
mod message_tests;
mod response_retry_tests;
