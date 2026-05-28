use super::*;

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
