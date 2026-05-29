use super::*;

#[tokio::test]
async fn send_waits_for_current_sender_typing_to_stop() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let mut msg = inbound();
    msg.gateway_id = "relay".into();
    msg.sender = Some(protocol::ObservedSender::primary(
        "relay", "local", None, "test",
    ));
    msg.channel = protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct);
    msg.conversation = ConversationId("relay:local".into());
    ensure_test_channel(store.as_ref(), "relay", "local", ChannelKind::Direct).await;
    let key = (
        msg.conversation.clone(),
        msg.gateway_id.clone(),
        msg.sender_external_id().unwrap().to_string(),
    );
    let (ctx, _inject_tx) = test_context(store, gateway, msg);
    ctx.typing
        .write()
        .unwrap()
        .insert(key.clone(), crate::core::tools::util::now());
    let typing = ctx.typing.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        typing.write().unwrap().remove(&key);
    });
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
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    let started = std::time::Instant::now();
    let result = send(&json!({"content": "hi"}), &ctx, &mut state).await;

    assert_eq!(result, "Message sent.");
    assert!(state.responded);
    assert!(started.elapsed() >= std::time::Duration::from_millis(100));
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &[("local".to_string(), "hi".to_string())]
    );
}
