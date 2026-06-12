use super::*;

#[tokio::test]
async fn typing_sender_does_not_defer_fresh_response() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    let person = PersonId("sam".into());
    let mut msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    msg.person = Some(person.clone());
    mind.update_typing(
        &msg.conversation,
        &msg.gateway_id,
        msg.sender_external_id().unwrap(),
        true,
    );

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::Message(msg),
        )
        .await;
    assert!(matches!(decision, MindDecision::Spawn(_)));
}
#[tokio::test]
async fn typing_stop_flushes_matching_deferred_typing_message() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (mind, mut flushed_rx) = test_mind_with_gateway_state_and_event_receiver(
        store.clone(),
        GatewayConnectionState::Connected,
    );
    let mut msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    msg.metadata = serde_json::json!({
        "mind_defer_count": 1,
        "mind_defer_reason": "typing",
    });
    let now = chrono::Utc::now().timestamp();
    store
        .enqueue_event(&EventInboxRecord {
            id: "typing-event".into(),
            kind: "message".into(),
            payload: serde_json::to_value(&msg).unwrap(),
            status: "pending".into(),
            due_at: now + 300,
            attempts: 0,
            dedupe_key: Some("message:relay:local-msg-1:1".into()),
            created_at: now,
            updated_at: now,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    mind.flush_deferred_typing_messages(
        &msg.conversation,
        &msg.gateway_id,
        msg.sender_external_id().unwrap(),
    )
    .await;

    let flushed = tokio::time::timeout(std::time::Duration::from_secs(1), flushed_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match flushed {
        WakeEvent::Message(flushed_msg) => assert_eq!(flushed_msg.message_id, "local-msg-1"),
        _ => panic!("expected flushed message"),
    }
    assert!(store.due_events(now + 301, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn typing_stop_marks_malformed_pending_message_failed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (mind, mut flushed_rx) = test_mind_with_gateway_state_and_event_receiver(
        store.clone(),
        GatewayConnectionState::Connected,
    );
    let now = chrono::Utc::now().timestamp();
    store
        .enqueue_event(&EventInboxRecord {
            id: "malformed-typing-event".into(),
            kind: "message".into(),
            payload: serde_json::json!({"malformed": true}),
            status: "pending".into(),
            due_at: now + 300,
            attempts: 0,
            dedupe_key: Some("message:relay:malformed:1".into()),
            created_at: now,
            updated_at: now,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    mind.flush_deferred_typing_messages(&ConversationId("relay:local".into()), "relay", "local")
        .await;

    assert!(flushed_rx.try_recv().is_err());
    assert!(
        store
            .pending_events_by_kind("message", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn typing_stop_keeps_pending_message_when_flush_channel_is_closed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (mind, flushed_rx) = test_mind_with_gateway_state_and_event_receiver(
        store.clone(),
        GatewayConnectionState::Connected,
    );
    drop(flushed_rx);
    let now = chrono::Utc::now().timestamp();
    let mut msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    msg.metadata = serde_json::json!({
        "mind_defer_reason": "typing",
    });
    store
        .enqueue_event(&EventInboxRecord {
            id: "typing-event".into(),
            kind: "message".into(),
            payload: serde_json::to_value(&msg).unwrap(),
            status: "pending".into(),
            due_at: now + 300,
            attempts: 0,
            dedupe_key: Some("message:relay:local:local-msg-1".into()),
            created_at: now,
            updated_at: now,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    mind.flush_deferred_typing_messages(&ConversationId("relay:local".into()), "relay", "local")
        .await;

    let pending = store.pending_events_by_kind("message", 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "typing-event");
}
#[tokio::test]
async fn idle_tick_prunes_stale_typing_state() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);
    let conversation = ConversationId("relay:local".into());
    let stale_key = (
        conversation.clone(),
        "relay".to_string(),
        "stale-sender".to_string(),
    );
    let active_key = (
        conversation.clone(),
        "relay".to_string(),
        "active-sender".to_string(),
    );
    let future_key = (
        conversation,
        "relay".to_string(),
        "future-sender".to_string(),
    );
    let now = recent_timestamp();
    {
        let mut typing = mind.typing.write().unwrap();
        typing.insert(stale_key.clone(), now - TYPING_ACTIVE_SECS - 1);
        typing.insert(active_key.clone(), now);
        typing.insert(future_key.clone(), now + 60);
    }

    let pruned = mind.prune_stale_typing(now);

    assert_eq!(pruned, 1);
    let typing = mind.typing.read().unwrap();
    assert!(!typing.contains_key(&stale_key));
    assert!(typing.contains_key(&active_key));
    assert!(typing.contains_key(&future_key));
}
#[test]
fn idle_tick_waits_for_actual_inactivity_window() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);

    assert!(mind.idle_tick_is_due(300.0));
    mind.record_activity();
    assert!(!mind.idle_tick_is_due(300.0));
    mind.last_activity_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(600));
    assert!(mind.idle_tick_is_due(300.0));
    assert!(!mind.idle_tick_is_due(0.0));

    assert!(!event_counts_as_activity(&WakeEvent::IdleTick {
        elapsed_secs: 300.0,
    }));
    assert!(event_counts_as_activity(&WakeEvent::TypingUpdate {
        conversation: ConversationId("relay:local".into()),
        gateway_id: "relay".into(),
        sender_external_id: "local".into(),
        typing: true,
    }));
}
