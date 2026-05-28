use super::*;

#[tokio::test]
async fn outreach_send_defaults_to_stored_conversation_reply_target() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let conv = ConversationId("relay:local".into());
    store
        .append_message(
            &conv,
            Some("relay"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "last inbound".into(),
                identity: None,
                profile: None,
                person: Some(PersonId("person-sam".into())),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: Value::Null,
            },
        )
        .await
        .unwrap();

    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let (mut ctx, _inject_tx) = test_context(store.clone(), gateway, inbound());
    ctx.kind = SessionKind::Action(ActionKind::Outreach);
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

    let result = send(&json!({"content": "checking in"}), &ctx, &mut state).await;

    assert_eq!(result, "Message sent.");
    assert!(state.responded);
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &[("local".to_string(), "checking in".to_string())]
    );
    assert_eq!(state.delta.relationship_changes.len(), 1);
    assert_eq!(
        state.delta.relationship_changes[0].person,
        PersonId("person-sam".into())
    );
    assert!(matches!(
        state.delta.relationship_changes[0].interaction,
        Some(RelationshipInteraction::ProactiveOutbound)
    ));
}
#[tokio::test]
async fn outreach_send_marks_relationship_delta_as_proactive_outbound() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(RecordingAdapter { sent: sent.clone() }));
    let mut msg = inbound();
    msg.gateway_id = "relay".into();
    msg.reply_external_id = "local".into();
    msg.conversation = ConversationId("relay:local".into());
    msg.person = Some(protocol::PersonId("person-sam".into()));
    let (mut ctx, _inject_tx) = test_context(store, gateway, msg);
    ctx.kind = SessionKind::Action(ActionKind::Outreach);
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

    let result = send(&json!({"content": "checking in"}), &ctx, &mut state).await;

    assert_eq!(result, "Message sent.");
    assert!(state.responded);
    assert!(matches!(
        state.delta.relationship_changes[0].interaction,
        Some(RelationshipInteraction::ProactiveOutbound)
    ));
}
