use super::*;

#[tokio::test]
async fn snapshots() {
    let store = test_store();
    let snapshot = ActorSnapshot {
        state: ActorState::new(CoreTraits::default()),
        config: GrowthConfig::default(),
        saved_at: 3000,
        last_state_journal_id: Some(7),
    };
    store.save_snapshot(&snapshot).await.unwrap();

    let loaded = store.load_latest_snapshot().await.unwrap().unwrap();
    assert_eq!(loaded.saved_at, 3000);
    assert_eq!(loaded.last_state_journal_id, Some(7));
}

#[tokio::test]
async fn state_journal_records_are_ordered_and_replayable() {
    let store = test_store();
    let first = store
        .append_state_journal("delta", &serde_json::json!({"growth_note": "first"}), 1000)
        .await
        .unwrap();
    let second = store
        .append_state_journal(
            "idle_tick",
            &serde_json::json!({"elapsed_secs": 300.0}),
            1001,
        )
        .await
        .unwrap();

    let records = store.state_journal_after(Some(first), 10).await.unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, second);
    assert_eq!(records[0].kind, "idle_tick");
    assert_eq!(records[0].payload["elapsed_secs"], 300.0);
}
