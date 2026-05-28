use super::messages::{
    inject_pending_messages, message_metadata, remember_injected_message, required_capabilities,
    source_message_keys,
};
use super::{SessionResult, build_result, run_session};
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::{SessionContext, SessionKind, SessionState, empty_delta};
use crate::state::{ActorState, Authority, GrowthConfig};
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
        authority: Authority::Default,
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

#[test]
fn message_metadata_embeds_attachments() {
    let metadata = message_metadata(&inbound(serde_json::json!({ "sender": "user" })));

    assert_eq!(metadata["sender"], "user");
    assert_eq!(metadata["attachments"][0]["kind"], "Sticker");
    assert_eq!(metadata["attachments"][0]["asset_id"], "media-1");
    assert_eq!(metadata["attachments"][0]["mime"], "image/webp");
}

#[test]
fn visual_attachments_require_vision() {
    let mut msg = inbound(Value::Null);
    msg.attachments[0].kind = MediaKind::Video;

    assert_eq!(required_capabilities(&[msg], &[]), vec![Capability::Vision]);
}

#[test]
fn file_attachments_do_not_require_vision() {
    let mut msg = inbound(Value::Null);
    msg.attachments[0].kind = MediaKind::File;

    assert!(required_capabilities(&[msg], &[]).is_empty());
}

#[test]
fn action_outcome_carries_review_artifacts() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let ctx = test_context(store, text_inbound("msg-1", "hello"));
    let thought = Thought {
        timestamp: 1000,
        kind: ThoughtKind::Observation,
        content: "Sam sounded rushed.".into(),
        importance: 0.8,
        confidence: 0.7,
        action_id: Some("action-test".into()),
        memories_accessed: vec![MemoryId("memory-recalled".into())],
        subjects: vec![],
    };
    let formed = MemoryId("memory-formed".into());
    let recalled = MemoryId("memory-recalled".into());
    let state = SessionState {
        responded: true,
        attempted_send: true,
        composing_released: false,
        delta: empty_delta(None),
        thoughts: vec![thought.clone()],
        memories_formed: vec![formed.clone()],
        recalled_memory_ids: vec![recalled.clone()],
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

    let result = build_result(ctx, state, None, 1, false);

    match result {
        SessionResult::Action(outcome) => {
            assert!(outcome.responded);
            assert_eq!(outcome.thoughts.len(), 1);
            assert_eq!(outcome.thoughts[0].content, thought.content);
            assert_eq!(outcome.memories_formed, vec![formed]);
            assert_eq!(outcome.recalled_memory_ids, vec![recalled]);
        }
        SessionResult::Mind(_) => panic!("expected action outcome"),
    }
}

#[test]
fn prompt_snapshot_redacts_prompt_text_tool_payloads_and_images() {
    let messages = vec![
        Message::system("System prompt with profile context."),
        Message::User(UserMessage::Content(vec![
            ContentPart::text("Please inspect this image."),
            ContentPart::image_url("data:image/png;base64,secret-bytes"),
        ])),
        Message::Assistant(AssistantMessage {
            text: Some("I will send the update.".into()),
            reasoning_content: Some("private reasoning".into()),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "send_message".into(),
                arguments: serde_json::json!({
                    "content": "Private reply text.",
                    "external_id": "target-external-id",
                    "metadata": {
                        "safe": true
                    }
                }),
            }],
        }),
        Message::tool_result(
            "call-1",
            r#"{"messages":[{"content":"private readback"}],"result":"Message sent."}"#,
        ),
    ];

    let snapshot = super::prompt_snapshot_messages(&messages);

    assert_eq!(snapshot[0]["content"], "[redacted]");
    assert_eq!(
        snapshot[0]["content_len"],
        "System prompt with profile context.".len()
    );
    assert_eq!(snapshot[1]["content"], "[redacted]");
    assert_eq!(snapshot[1]["content_parts"][0]["content"], "[redacted]");
    assert_eq!(
        snapshot[1]["content_parts"][0]["content_len"],
        "Please inspect this image.".len()
    );
    assert_eq!(
        snapshot[1]["content_parts"][1]["url"],
        "[inline image redacted]"
    );
    assert_eq!(snapshot[2]["content"], "[redacted]");
    assert_eq!(snapshot[2]["content_len"], "I will send the update.".len());
    assert_eq!(
        snapshot[2]["tool_calls"][0]["arguments"]["content"],
        "[redacted]"
    );
    assert_eq!(
        snapshot[2]["tool_calls"][0]["arguments"]["external_id"],
        "[redacted]"
    );
    assert_eq!(
        snapshot[2]["tool_calls"][0]["arguments"]["metadata"]["safe"],
        true
    );
    assert_eq!(snapshot[2]["reasoning_len"], "private reasoning".len());
    assert!(snapshot[2].get("reasoning_content").is_none());
    assert_eq!(
        snapshot[3]["content"]["messages"][0]["content"],
        "[redacted]"
    );
}

#[tokio::test]
async fn injected_messages_dedupe_by_source_id_not_text() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let source = text_inbound("msg-1", "same text");
    let ctx = test_context(store.clone(), source.clone());
    let mut state = SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: empty_delta(None),
        thoughts: vec![],
        memories_formed: vec![],
        recalled_memory_ids: vec![],
        injected_messages: vec![],
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: source_message_keys(&ctx.messages),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };
    let mut llm_messages = vec![Message::system("system"), Message::user("same text")];

    let injected_a = text_inbound("msg-2", "same text");
    let injected_b = text_inbound("msg-3", "same text");
    let duplicate_injected_a = text_inbound("msg-2", "same text");
    let duplicate_source = text_inbound("msg-1", "same text");

    assert!(remember_injected_message(&mut state, injected_a));
    assert!(remember_injected_message(&mut state, injected_b));
    assert!(!remember_injected_message(&mut state, duplicate_injected_a));
    assert!(!remember_injected_message(&mut state, duplicate_source));

    inject_pending_messages(&ctx, &mut state, &mut llm_messages).await;

    let same_text_user_messages = llm_messages
        .iter()
        .filter(
            |message| matches!(message, Message::User(user) if user.display_text() == "same text"),
        )
        .count();
    assert_eq!(same_text_user_messages, 3);
    assert_eq!(state.presented_injection_count, 2);
    assert_eq!(state.presented_injected_messages.len(), 2);
    assert_eq!(state.presented_injected_messages[0].message_id, "msg-2");
    assert_eq!(state.presented_injected_messages[1].message_id, "msg-3");
    assert!(state.pending_injected_messages.is_empty());

    let stored = store
        .get_messages(&source.conversation, 10, None)
        .await
        .unwrap();
    let source_ids = stored
        .iter()
        .filter(|message| matches!(message.role, MessageRole::User))
        .filter_map(|message| message.source_message_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(source_ids, vec!["msg-2", "msg-3"]);
}

#[tokio::test]
async fn composing_is_released_when_session_task_is_aborted() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut ctx = test_context(store, text_inbound("msg-1", "hello"));
    let gateway = Arc::new(GatewayRouter::new());
    let router = Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(HangingBridge)),
                model: "hang".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    );
    ctx.gateway = gateway.clone();
    ctx.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    ctx.router = router;

    let handle = tokio::spawn(run_session(ctx));
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 1).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 1);

    handle.abort();
    match handle.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("session unexpectedly completed"),
    }
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 0).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 0);
}

#[tokio::test]
async fn cooperative_cancellation_releases_composing_without_abort() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut ctx = test_context(store, text_inbound("msg-1", "hello"));
    let gateway = Arc::new(GatewayRouter::new());
    let router = Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(HangingBridge)),
                model: "hang".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    );
    ctx.gateway = gateway.clone();
    ctx.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    ctx.router = router;
    let progress = ctx.progress.clone();

    let handle = tokio::spawn(run_session(ctx));
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 1).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 1);

    progress.read().unwrap().request_cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("cooperative cancellation should finish the session")
        .expect("session task should not be aborted");

    match result {
        SessionResult::Action(outcome) => {
            assert!(outcome.cancelled);
            assert!(!outcome.responded);
        }
        SessionResult::Mind(_) => panic!("expected action outcome"),
    }
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 0).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 0);
}

#[tokio::test]
async fn aborting_one_of_two_sessions_preserves_shared_composing_reference() {
    let gateway = Arc::new(GatewayRouter::new());
    let router = Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(HangingBridge)),
                model: "hang".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    );

    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut first = test_context(store.clone(), text_inbound("msg-1", "hello"));
    first.action_id = ActionId("action-first".into());
    first.gateway = gateway.clone();
    first.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    first.router = router.clone();

    let mut second = test_context(store, text_inbound("msg-2", "hello again"));
    second.action_id = ActionId("action-second".into());
    second.gateway = gateway.clone();
    second.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    second.router = router;

    let first_handle = tokio::spawn(run_session(first));
    let second_handle = tokio::spawn(run_session(second));
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 2).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 2);

    first_handle.abort();
    match first_handle.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("first session unexpectedly completed"),
    }
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 1).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 1);

    second_handle.abort();
    match second_handle.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("second session unexpectedly completed"),
    }
    wait_for_composing_count(&gateway, "whatsapp", "chat-1", 0).await;
    assert_eq!(gateway.composing_count("whatsapp", "chat-1").await, 0);
}

#[tokio::test]
async fn response_action_retries_when_model_emits_text_without_tool_call() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let bridge = Arc::new(TextThenSendBridge {
        calls: AtomicUsize::new(0),
    });
    let router = Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(bridge.clone()),
                model: "scripted".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    );
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let mut ctx = test_context(store, text_inbound("msg-1", "hello"));
    ctx.router = router.clone();
    ctx.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    ctx.gateway = gateway;
    ctx.max_action_attempts = 2;

    let result = run_session(ctx).await;

    match result {
        SessionResult::Action(outcome) => {
            assert!(outcome.responded);
            assert_eq!(outcome.attempts, 2);
        }
        SessionResult::Mind(_) => panic!("expected action outcome"),
    }
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["visible reply".to_string()]
    );
}

#[tokio::test]
async fn response_action_reemits_injected_messages_not_presented_before_send() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let bridge = Arc::new(SendOnlyBridge {
        calls: AtomicUsize::new(0),
    });
    let router = Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(bridge.clone()),
                model: "scripted".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    );
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));

    let mut ctx = test_context(store, text_inbound("msg-1", "hello"));
    ctx.router = router.clone();
    ctx.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    ctx.gateway = gateway;
    let (inject_tx, inject_rx) = mpsc::channel(2);
    inject_tx
        .send(text_inbound("msg-2", "one more thing"))
        .await
        .unwrap();
    drop(inject_tx);
    ctx.inject_rx = inject_rx;

    let result = run_session(ctx).await;

    match result {
        SessionResult::Action(outcome) => {
            assert!(outcome.responded);
            assert_eq!(outcome.pending_messages.len(), 1);
            assert_eq!(outcome.pending_messages[0].message_id, "msg-2");
            assert!(outcome.review_messages.is_empty());
            assert!(!outcome.had_injections);
        }
        SessionResult::Mind(_) => panic!("expected action outcome"),
    }
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["visible reply".to_string()]
    );
}

#[tokio::test]
async fn response_action_stops_retrying_after_delivery_attempt_fails() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let bridge = Arc::new(SendOnlyBridge {
        calls: AtomicUsize::new(0),
    });
    let router = Arc::new(
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(bridge.clone()),
                model: "scripted".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap(),
    );
    let mut ctx = test_context(store.clone(), text_inbound("msg-1", "hello"));
    ctx.router = router.clone();
    ctx.endpoints = router.resolve_chain(&RouteContext::Action(Reasoning::Basic));
    ctx.max_action_attempts = 3;

    let result = run_session(ctx).await;

    match result {
        SessionResult::Action(outcome) => {
            assert!(!outcome.responded);
            assert!(outcome.attempted_send);
            assert_eq!(outcome.attempts, 1);
        }
        SessionResult::Mind(_) => panic!("expected action outcome"),
    }
    assert_eq!(bridge.calls.load(Ordering::SeqCst), 1);
    let deliveries = store
        .outbound_deliveries_for_action("action-test")
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, "failed");
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
