use super::*;

#[tokio::test]
async fn event_inbox_persists_due_events_and_fires_once() {
    let store = test_store();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-1".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message_id": "msg-1"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("message:relay:msg-1:1".into()),
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-duplicate".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message_id": "msg-1"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("message:relay:msg-1:1".into()),
            created_at: 901,
            updated_at: 901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    assert!(store.due_events(999, 10).await.unwrap().is_empty());
    let due = store.due_events(1000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "event-1");
    assert_eq!(due[0].payload["message_id"], "msg-1");

    assert!(store.mark_event_fired("event-1", 1001).await.unwrap());
    assert!(!store.mark_event_fired("event-1", 1002).await.unwrap());
    assert!(store.due_events(2000, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn due_events_coalesce_message_events_by_conversation_per_scan() {
    let store = test_store();
    for (id, conversation, created_at) in [
        ("event-a-1", "relay:a", 900),
        ("event-a-2", "relay:a", 901),
        ("event-b-1", "relay:b", 902),
    ] {
        store
            .enqueue_event(&EventInboxRecord {
                id: id.into(),
                kind: "message".into(),
                payload: serde_json::json!({
                    "message_id": id.replace("event-", "msg-"),
                    "conversation": conversation,
                }),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: Some(format!("message:{id}")),
                created_at,
                updated_at: created_at,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();
    }

    let due = store.due_events(1000, 10).await.unwrap();
    let ids = due
        .iter()
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["event-a-1", "event-b-1"]);

    assert!(store.mark_event_fired("event-a-1", 1001).await.unwrap());
    let due = store.due_events(1001, 10).await.unwrap();
    let ids = due
        .iter()
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["event-a-2", "event-b-1"]);
}

#[tokio::test]
async fn event_inbox_failed_events_leave_pending_queue_once() {
    let store = test_store();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-bad".into(),
            kind: "message".into(),
            payload: serde_json::json!({"malformed": true}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("message:bad".into()),
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    assert!(
        store
            .mark_event_failed("event-bad", 1001, Some("malformed test payload"))
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_event_failed("event-bad", 1002, Some("second failure ignored"))
            .await
            .unwrap()
    );
    assert!(store.due_events(2000, 10).await.unwrap().is_empty());

    let conn = store.lock().unwrap();
    let (status, attempts, updated_at, fired_at, last_error): (
        String,
        u32,
        i64,
        Option<i64>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT status, attempts, updated_at, fired_at, last_error FROM event_inbox WHERE id = ?1",
            params!["event-bad"],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1);
    assert_eq!(updated_at, 1001);
    assert_eq!(fired_at, None);
    assert_eq!(last_error.as_deref(), Some("malformed test payload"));
}

#[tokio::test]
async fn event_inbox_surfaces_malformed_payload_rows_for_failure_handling() {
    let store = test_store();
    {
        let conn = store.lock().unwrap();
        conn.execute(
            "INSERT INTO event_inbox (
                id, kind, payload_json, status, due_at, attempts, dedupe_key,
                created_at, updated_at, fired_at, last_error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "event-corrupt-payload",
                "message",
                "{not-json",
                "pending",
                1000,
                0_u32,
                Option::<String>::None,
                900,
                900,
                Option::<i64>::None,
                Option::<String>::None,
            ],
        )
        .unwrap();
    }

    let due = store.due_events(1000, 10).await.unwrap();

    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "event-corrupt-payload");
    assert!(due[0].payload.is_null());
    assert!(
        due[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("malformed event payload json"))
    );
    assert!(
        store
            .mark_event_failed("event-corrupt-payload", 1001, due[0].last_error.as_deref())
            .await
            .unwrap()
    );

    let failed = store.debug_recent_failed_events(10).await.unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].id, "event-corrupt-payload");
    assert!(
        failed[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("malformed event payload json"))
    );
}

#[tokio::test]
async fn event_inbox_lists_pending_events_by_kind_before_due_time() {
    let store = test_store();
    store
        .enqueue_event(&EventInboxRecord {
            id: "message-event".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message_id": "msg-1"}),
            status: "pending".into(),
            due_at: 2000,
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
            id: "intent-event".into(),
            kind: "intent_fired".into(),
            payload: serde_json::json!({"id": "intent-1"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 901,
            updated_at: 901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    assert!(store.due_events(999, 10).await.unwrap().is_empty());
    let pending = store.pending_events_by_kind("message", 10).await.unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "message-event");
    assert_eq!(pending[0].payload["message_id"], "msg-1");
}
