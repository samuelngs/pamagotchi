use super::*;

#[tokio::test]
async fn scheduler_drains_due_message_events_once() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let message = inbound();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-message".into(),
            kind: "message".into(),
            payload: serde_json::to_value(&message).unwrap(),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 800,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel(1);
    assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

    match rx.recv().await.unwrap() {
        WakeEvent::Message(msg) => assert_eq!(msg.message_id, "msg-1"),
        _ => panic!("expected deferred message event"),
    }
    assert!(store.due_events(1001, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn claimed_persisted_event_is_not_emitted_twice() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let message = inbound();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-message".into(),
            kind: "message".into(),
            payload: serde_json::to_value(&message).unwrap(),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    assert!(store.mark_event_fired("event-message", 1000).await.unwrap());

    let (tx, mut rx) = mpsc::channel(1);
    assert!(
        claim_and_send_persisted_event(
            &tx,
            store_dyn.as_ref(),
            "event-message",
            1001,
            WakeEvent::Message(message),
            "test duplicate"
        )
        .await
    );

    assert!(rx.try_recv().is_err());
    assert!(store.due_events(1001, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn persisted_event_stays_pending_when_handoff_channel_is_closed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let message = inbound();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-message".into(),
            kind: "message".into(),
            payload: serde_json::to_value(&message).unwrap(),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    let (tx, rx) = mpsc::channel(1);
    drop(rx);

    assert!(
        !claim_and_send_persisted_event(
            &tx,
            store_dyn.as_ref(),
            "event-message",
            1000,
            WakeEvent::Message(message),
            "test closed channel"
        )
        .await
    );

    let due = store.due_events(1000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "event-message");
}
#[tokio::test]
async fn scheduler_leaves_due_event_pending_when_actor_channel_is_closed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let message = inbound();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-message".into(),
            kind: "message".into(),
            payload: serde_json::to_value(&message).unwrap(),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    assert!(!drain_due_events(&tx, store_dyn, 1000, 10).await);

    let due = store.due_events(1001, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "event-message");
}
#[tokio::test]
async fn scheduler_replays_message_edit_and_delete_events() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let conversation = ConversationId("relay:local".into());
    let edited = crate::core::MessageEditedEvent {
        conversation: conversation.clone(),
        gateway_id: "relay".into(),
        message_id: "msg-1".into(),
        content: "edited text".into(),
        edited_at: 1100,
    };
    let deleted = crate::core::MessageDeletedEvent {
        conversation: conversation.clone(),
        gateway_id: "relay".into(),
        message_id: "msg-2".into(),
        deleted_at: 1200,
    };
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-edit".into(),
            kind: "message_edited".into(),
            payload: serde_json::to_value(&edited).unwrap(),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-delete".into(),
            kind: "message_deleted".into(),
            payload: serde_json::to_value(&deleted).unwrap(),
            status: "pending".into(),
            due_at: 1001,
            attempts: 0,
            dedupe_key: None,
            created_at: 901,
            updated_at: 901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel(2);
    assert!(drain_due_events(&tx, store_dyn, 1001, 10).await);

    match rx.recv().await.unwrap() {
        WakeEvent::MessageEdited {
            conversation,
            gateway_id,
            message_id,
            content,
            edited_at,
        } => {
            assert_eq!(conversation.0, "relay:local");
            assert_eq!(gateway_id, "relay");
            assert_eq!(message_id, "msg-1");
            assert_eq!(content, "edited text");
            assert_eq!(edited_at, 1100);
        }
        _ => panic!("expected message edit event"),
    }
    match rx.recv().await.unwrap() {
        WakeEvent::MessageDeleted {
            conversation,
            gateway_id,
            message_id,
            deleted_at,
        } => {
            assert_eq!(conversation.0, "relay:local");
            assert_eq!(gateway_id, "relay");
            assert_eq!(message_id, "msg-2");
            assert_eq!(deleted_at, 1200);
        }
        _ => panic!("expected message delete event"),
    }
    assert!(store.due_events(1002, 10).await.unwrap().is_empty());
}
