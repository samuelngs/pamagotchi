use super::*;

#[tokio::test]
async fn scheduler_drains_due_consolidation_events_once() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-consolidation".into(),
            kind: "consolidation_due".into(),
            payload: serde_json::json!({}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("consolidation-due".into()),
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel(1);
    assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

    assert!(matches!(rx.recv().await, Some(WakeEvent::ConsolidationDue)));
    assert!(store.due_events(1001, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn scheduler_marks_malformed_due_events_failed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-malformed-message".into(),
            kind: "message".into(),
            payload: serde_json::json!({"malformed": true}),
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

    let (tx, mut rx) = mpsc::channel(1);
    assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

    assert!(rx.try_recv().is_err());
    assert!(store.due_events(1001, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn scheduler_emits_due_consolidation_event() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (tx, mut rx) = mpsc::channel(1);

    assert!(emit_due_consolidation(&tx, store_dyn, 1000).await);
    assert!(matches!(rx.recv().await, Some(WakeEvent::ConsolidationDue)));
    assert!(
        store
            .pending_events_by_kind("consolidation_due", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn scheduler_leaves_periodic_consolidation_pending_when_actor_channel_is_closed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (tx, rx) = mpsc::channel(1);
    drop(rx);

    assert!(!emit_due_consolidation(&tx, store_dyn, 1000).await);
    let pending = store
        .pending_events_by_kind("consolidation_due", 10)
        .await
        .unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, "consolidation_due");
    assert_eq!(pending[0].dedupe_key.as_deref(), Some("consolidation-due"));
}
#[tokio::test]
async fn scheduler_does_not_duplicate_pending_periodic_consolidation() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-consolidation-existing".into(),
            kind: "consolidation_due".into(),
            payload: serde_json::json!({}),
            status: "pending".into(),
            due_at: 2000,
            attempts: 0,
            dedupe_key: Some("consolidation-due".into()),
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    let (tx, mut rx) = mpsc::channel(1);

    assert!(emit_due_consolidation(&tx, store_dyn, 1000).await);

    assert!(rx.try_recv().is_err());
    let pending = store
        .pending_events_by_kind("consolidation_due", 10)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "event-consolidation-existing");
}
#[test]
fn scheduler_elapsed_uses_actual_monotonic_gap() {
    let mut elapsed = 0.0;

    assert_eq!(take_due_scheduler_elapsed(&mut elapsed, 30.0, 300.0), None);
    assert_eq!(elapsed, 30.0);
    assert_eq!(
        take_due_scheduler_elapsed(&mut elapsed, 420.0, 300.0),
        Some(450.0)
    );
    assert_eq!(elapsed, 0.0);
}
