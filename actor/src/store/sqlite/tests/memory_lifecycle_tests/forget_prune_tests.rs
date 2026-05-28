use super::*;

#[tokio::test]
async fn memory_forget() {
    let store = test_store();
    store
        .store_memory(&sample_memory("m1", "gone", vec![0.1, 0.2, 0.3, 0.4]))
        .await
        .unwrap();

    assert!(store.forget(&MemoryId("m1".into())).await.unwrap());
    assert!(
        store
            .get_memory(&MemoryId("m1".into()))
            .await
            .unwrap()
            .is_none()
    );
    let conn = store.lock().unwrap();
    let operation: String = conn
        .query_row(
            "SELECT operation FROM memory_mutations WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params!["m1"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation, "forget");
    drop(conn);

    assert!(!store.forget(&MemoryId("m1".into())).await.unwrap());

    store
        .store_memory(&sample_memory(
            "m2",
            "chosen_person deleted",
            vec![0.4, 0.3, 0.2, 0.1],
        ))
        .await
        .unwrap();
    assert!(
        store
            .forget_with_reason(
                &MemoryId("m2".into()),
                Some("chosen_person requested deletion")
            )
            .await
            .unwrap()
    );
    let conn = store.lock().unwrap();
    let reason: Option<String> = conn
        .query_row(
            "SELECT reason FROM memory_mutations WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params!["m2"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason.as_deref(), Some("chosen_person requested deletion"));
    drop(conn);

    let mutations = store
        .memory_mutations_for_memory(&MemoryId("m2".into()), 10)
        .await
        .unwrap();
    assert_eq!(mutations[0].operation, "forget");
    assert_eq!(
        mutations[0].reason.as_deref(),
        Some("chosen_person requested deletion")
    );
    assert!(
        mutations
            .iter()
            .any(|mutation| mutation.operation == "create")
    );
}
#[tokio::test]
async fn prune_stale_memories_removes_only_low_signal_expired_or_superseded_rows() {
    let store = test_store();
    let mut expired_low_signal =
        sample_memory("expired-low", "expired noise", vec![1.0, 0.0, 0.0, 0.0]);
    expired_low_signal.importance = 0.1;
    expired_low_signal.confidence = 0.2;
    expired_low_signal.sensitivity = 0.1;
    expired_low_signal.expires_at = Some(900);
    expired_low_signal.accessed_at = 800;

    let mut superseded_low_signal =
        sample_memory("superseded-low", "old duplicate", vec![0.0, 1.0, 0.0, 0.0]);
    superseded_low_signal.importance = 0.1;
    superseded_low_signal.confidence = 0.2;
    superseded_low_signal.sensitivity = 0.1;
    superseded_low_signal.superseded_by = Some(MemoryId("replacement".into()));
    superseded_low_signal.accessed_at = 100;

    let mut active_low_signal =
        sample_memory("active-low", "active but low", vec![0.0, 0.0, 1.0, 0.0]);
    active_low_signal.importance = 0.1;
    active_low_signal.confidence = 0.2;
    active_low_signal.sensitivity = 0.1;
    active_low_signal.accessed_at = 100;

    let mut important_expired = sample_memory(
        "important-expired",
        "important expired",
        vec![0.0, 0.0, 0.0, 1.0],
    );
    important_expired.importance = 0.9;
    important_expired.confidence = 0.2;
    important_expired.sensitivity = 0.1;
    important_expired.expires_at = Some(900);

    let mut protected_boundary = sample_memory(
        "protected-boundary",
        "do not contact after 9pm",
        vec![0.5, 0.0, 0.0, 0.0],
    );
    protected_boundary.memory_type = MemoryType::Boundary;
    protected_boundary.importance = 0.1;
    protected_boundary.confidence = 0.2;
    protected_boundary.sensitivity = 0.1;
    protected_boundary.expires_at = Some(900);

    let mut secret_expired =
        sample_memory("secret-expired", "secret expired", vec![0.0, 0.5, 0.0, 0.0]);
    secret_expired.importance = 0.1;
    secret_expired.confidence = 0.2;
    secret_expired.sensitivity = 0.1;
    secret_expired.privacy_category = PrivacyCategory::Secret;
    secret_expired.expires_at = Some(900);

    let mut sensitive_expired = sample_memory(
        "sensitive-expired",
        "sensitive expired health reminder",
        vec![0.0, 0.0, 0.5, 0.0],
    );
    sensitive_expired.importance = 0.1;
    sensitive_expired.confidence = 0.2;
    sensitive_expired.sensitivity = 0.1;
    sensitive_expired.privacy_category = PrivacyCategory::Sensitive;
    sensitive_expired.expires_at = Some(900);

    for memory in [
        expired_low_signal,
        superseded_low_signal,
        active_low_signal,
        important_expired,
        protected_boundary,
        secret_expired,
        sensitive_expired,
    ] {
        store.store_memory(&memory).await.unwrap();
    }

    let pruned = store
        .prune_stale_memories(1000, 500, 0.3, 0.5, 0.4, 100)
        .await
        .unwrap();
    assert_eq!(pruned, 2);
    assert!(
        store
            .get_memory(&MemoryId("expired-low".into()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_memory(&MemoryId("superseded-low".into()))
            .await
            .unwrap()
            .is_none()
    );
    for id in [
        "active-low",
        "important-expired",
        "protected-boundary",
        "secret-expired",
        "sensitive-expired",
    ] {
        assert!(
            store
                .get_memory(&MemoryId(id.into()))
                .await
                .unwrap()
                .is_some(),
            "{id} should be retained"
        );
    }

    let conn = store.lock().unwrap();
    let operation: String = conn
        .query_row(
            "SELECT operation FROM memory_mutations WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params!["expired-low"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation, "prune");
}
