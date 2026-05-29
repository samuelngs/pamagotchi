use super::*;

#[tokio::test]
async fn send_normalizes_em_dash_before_delivery_and_storage() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let mut msg = inbound();
    msg.gateway_id = "relay".into();
    msg.reply_external_id = "local".into();
    msg.conversation = ConversationId("relay:local".into());
    let conv = msg.conversation.clone();
    let (ctx, _inject_tx) = test_context(store.clone(), gateway, msg);
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

    let result = send(&json!({"content": "wait\u{2014}hold on"}), &ctx, &mut state).await;

    assert_eq!(result, "Message sent.");
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &[("local".to_string(), "wait, hold on".to_string())]
    );
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "wait, hold on");
}

#[tokio::test]
async fn failed_delivery_does_not_mark_response_delivered() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let conv = ConversationId("missing-gateway:reply-target".into());
    let (ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
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

    let result = send(&json!({"content": "hi"}), &ctx, &mut state).await;

    assert!(!state.responded);
    assert!(state.attempted_send);
    assert!(result.contains("not added to visible conversation history"));
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert!(messages.is_empty());
    let deliveries = store
        .outbound_deliveries_for_action("action-test")
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].conversation.as_ref(), Some(&conv));
    assert_eq!(deliveries[0].gateway_id, "missing-gateway");
    assert_eq!(deliveries[0].external_id, "reply-target");
    assert_eq!(deliveries[0].status, "failed");
    assert!(deliveries[0].error.is_some());
    assert!(store.due_intents(i64::MAX, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn failed_delivery_schedules_deduped_chosen_human_review_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let chosen_human = PersonId("person-chosen_human".into());
    let (ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
    ctx.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&chosen_human, Some(Authority::ChosenHuman));
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

    let result = send(&json!({"content": "hi"}), &ctx, &mut state).await;
    let result_again = send(&json!({"content": "hi again"}), &ctx, &mut state).await;

    assert!(result.contains("Chosen-human review is queued"));
    assert!(result_again.contains("Chosen-human review is queued"));
    let intents = store.due_intents(i64::MAX, 10).await.unwrap();
    assert_eq!(intents.len(), 1);
    let intent = &intents[0];
    assert_eq!(intent.person.as_ref(), Some(&chosen_human));
    assert!(intent.chosen_human_approved);
    assert_eq!(intent.priority, 100);
    assert_eq!(intent.source_action.as_deref(), Some("action-test"));
    assert_eq!(
        intent.dedupe_key.as_deref(),
        Some("delivery-failure-review:action-test:missing-gateway:reply-target")
    );
    assert!(intent.task.contains("failed outbound delivery"));
    assert!(intent.task.contains("missing-gateway:reply-target"));
    assert!(intent.task.contains("Message length: 2 chars"));
}
