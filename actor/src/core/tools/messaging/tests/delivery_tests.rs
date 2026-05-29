use super::*;

#[tokio::test]
async fn send_normalizes_em_dash_before_delivery_and_storage() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let mut msg = inbound();
    msg.gateway_id = "relay".into();
    msg.channel = protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct);
    msg.conversation = ConversationId("relay:local".into());
    let channel = ensure_test_channel(store.as_ref(), "relay", "local", ChannelKind::Direct).await;
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
    assert_eq!(messages[0].metadata["channel_id"], channel.0);
    assert!(messages[0].metadata["message_id"].as_str().is_some());
}

#[tokio::test]
async fn failed_delivery_does_not_mark_response_delivered() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let conv = ConversationId("missing-gateway:reply-target".into());
    ensure_test_channel(
        store.as_ref(),
        "missing-gateway",
        "reply-target",
        ChannelKind::Direct,
    )
    .await;
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
    assert_eq!(
        deliveries[0].channel.as_ref(),
        Some(&channel_id(
            &GatewayId("missing-gateway".into()),
            "reply-target"
        ))
    );
    assert_eq!(deliveries[0].gateway_id, "missing-gateway");
    assert_eq!(deliveries[0].external_id, "reply-target");
    assert_eq!(deliveries[0].status, "failed");
    assert!(deliveries[0].error.is_some());
    assert!(store.due_intents(i64::MAX, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn default_delivery_uses_channel_record_for_stored_conversation() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let gateway_id = GatewayId("relay".into());
    let now = 1000;
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway_id.clone(),
            kind: "relay".into(),
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let channel = ChannelRecord {
        id: channel_id(&gateway_id, "local"),
        gateway: gateway_id.clone(),
        external_id: "local".into(),
        kind: ChannelKind::RelayRoom,
        space: None,
        parent: None,
        display_name: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    };
    store.upsert_channel(&channel).await.unwrap();
    let conv = store
        .get_or_create_active_conversation(&channel.id, now)
        .await
        .unwrap();

    let (mut ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
    ctx.messages.clear();
    ctx.conversation = Some(conv.clone());
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

    let result = send(&json!({"content": "from channel"}), &ctx, &mut state).await;

    assert_eq!(result, "Message sent.");
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &[("local".to_string(), "from channel".to_string())]
    );
    let deliveries = store
        .outbound_deliveries_for_action("action-test")
        .await
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].conversation.as_ref(), Some(&conv));
    assert_eq!(deliveries[0].channel.as_ref(), Some(&channel.id));
    assert!(deliveries[0].message.is_some());
    assert_eq!(deliveries[0].gateway_id, "relay");
    assert_eq!(deliveries[0].external_id, "local");
    assert_eq!(deliveries[0].status, "delivered");
}

#[tokio::test]
async fn default_delivery_does_not_fallback_to_conversation_person() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let conv = ConversationId("legacy-person-only".into());
    store
        .append_message(
            &conv,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::System,
                content: "person context only".into(),
                identity: None,
                profile: None,
                person: Some(PersonId("person-target".into())),
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: None,
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

    let (mut ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
    ctx.messages.clear();
    ctx.conversation = Some(conv);
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

    let result = send(&json!({"content": "should not send"}), &ctx, &mut state).await;

    assert_eq!(result, "No delivery target — message not sent.");
    assert!(!state.attempted_send);
    assert!(sent.lock().unwrap().is_empty());
    assert!(
        store
            .outbound_deliveries_for_action("action-test")
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn failed_delivery_schedules_deduped_chosen_human_review_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let chosen_human = PersonId("person-chosen_human".into());
    ensure_test_channel(
        store.as_ref(),
        "missing-gateway",
        "reply-target",
        ChannelKind::Direct,
    )
    .await;
    let (ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
    ctx.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&chosen_human, Some(RelationshipStanding::ChosenHuman));
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
