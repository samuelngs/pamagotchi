use super::*;

#[tokio::test]
async fn read_messages_includes_source_message_ids_for_review_evidence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let msg = inbound();
    store
        .append_message(
            &msg.conversation,
            Some("missing-gateway"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "source-backed message".into(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: Some("missing-gateway".into()),
                source_message_id: Some("source-msg-1".into()),
                sender_external_id: Some("sender-1".into()),
                reply_external_id: Some("reply-target".into()),
                metadata: Value::Null,
            },
        )
        .await
        .unwrap();
    let (ctx, _inject_tx) = test_context(store, gateway, msg);
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

    let result = read_with_state(&json!({"limit": 5}), &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["messages"][0]["message_id"], "source-msg-1");
    assert_eq!(
        parsed["messages"][0]["source"]["gateway_id"],
        "missing-gateway"
    );
    assert_eq!(
        parsed["messages"][0]["source"]["message_id"],
        "source-msg-1"
    );
    assert_eq!(state.presented_read_messages.len(), 1);
    assert_eq!(state.presented_read_messages[0].message_id, "source-msg-1");
    assert_eq!(
        state.presented_read_messages[0].gateway_id,
        "missing-gateway"
    );
    assert_eq!(
        state.presented_read_messages[0].sender_external_id,
        "sender-1"
    );

    let _ = read_with_state(&json!({"limit": 5}), &ctx, &mut state).await;
    assert_eq!(state.presented_read_messages.len(), 1);
}
#[tokio::test]
async fn read_messages_assigns_local_ids_to_unsourced_messages_for_summary_coverage() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let msg = inbound();
    store
        .append_message(
            &msg.conversation,
            Some("missing-gateway"),
            None,
            &StoredMessage {
                timestamp: 1001,
                role: MessageRole::Assistant,
                content: "visible assistant reply".into(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some("reply-target".into()),
                metadata: Value::Null,
            },
        )
        .await
        .unwrap();
    let (ctx, _inject_tx) = test_context(store.clone(), gateway, msg);

    let first_read = read(&json!({"limit": 5}), &ctx).await;
    let second_read = read(&json!({"limit": 5}), &ctx).await;
    let first: Value = serde_json::from_str(&first_read).unwrap();
    let second: Value = serde_json::from_str(&second_read).unwrap();
    let message_id = first["messages"][0]["message_id"].as_str().unwrap();

    assert!(message_id.starts_with("local:assistant:1001:"));
    assert_eq!(second["messages"][0]["message_id"], message_id);

    let result = update_conversation_summary(
        &json!({
            "summary": "Assistant replied visibly.",
            "covered_message_ids": [message_id]
        }),
        &ctx,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "updated");

    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(conversations[0].message_count, 1);
    assert_eq!(
        conversations[0].summary_covered_message_ids,
        vec![message_id.to_string()]
    );
    assert_eq!(
        conversations[0]
            .message_count
            .saturating_sub(conversations[0].summary_covered_message_ids.len() as u32),
        0
    );
}
#[tokio::test]
async fn ruminate_reads_recent_messages_without_current_conversation() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let newer = ConversationId("relay:newer".into());
    let older = ConversationId("relay:older".into());
    for (conversation, timestamp, content, source_message_id) in [
        (&older, 1000, "older context", "older-msg"),
        (&newer, 2000, "newer context", "newer-msg"),
    ] {
        store
            .append_message(
                conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp,
                    role: MessageRole::User,
                    content: content.into(),
                    identity: None,
                    profile: None,
                    person: None,
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some(source_message_id.into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: Value::Null,
                },
            )
            .await
            .unwrap();
    }

    let (mut ctx, _inject_tx) = test_context(store, gateway, inbound());
    ctx.kind = SessionKind::Action(ActionKind::Ruminate);
    ctx.messages.clear();
    ctx.conversation = None;

    let result = read(&json!({"limit": 4}), &ctx).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let conversations = parsed["conversations"].as_array().unwrap();

    assert_eq!(conversations.len(), 2);
    assert_eq!(conversations[0]["conversation"], "relay:newer");
    assert_eq!(conversations[0]["messages"][0]["message_id"], "newer-msg");
    assert_eq!(conversations[1]["conversation"], "relay:older");
    assert_eq!(conversations[1]["messages"][0]["content"], "older context");
}
#[tokio::test]
async fn default_action_without_current_conversation_cannot_read_recent_messages() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let gateway = Arc::new(GatewayRouter::new());
    let (mut ctx, _inject_tx) = test_context(store, gateway, inbound());
    ctx.messages.clear();
    ctx.conversation = None;

    let result = read(&json!({"limit": 4}), &ctx).await;

    assert_eq!(
        result,
        "No conversation specified and no current conversation."
    );
}
