use super::*;

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
