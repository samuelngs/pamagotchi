use super::*;

#[tokio::test]
async fn memory_dedupe_key_upserts_existing_memory_subjects_and_index() {
    let store = test_store();
    let mut original = sample_memory(
        "memory-original",
        "Sam prefers short status updates",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    original.dedupe_key = Some("preference:profile-sam:status-length".into());
    original.subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam".into()),
        Some("about".into()),
        1.0,
    )];
    let original_id = store.store_memory(&original).await.unwrap();

    let mut replacement = sample_memory(
        "memory-replacement",
        "Sam prefers concise deployment status updates",
        vec![0.2, 0.3, 0.4, 0.5],
    );
    replacement.dedupe_key = Some("preference:profile-sam:status-length".into());
    replacement.importance = 0.95;
    replacement.subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam-work".into()),
        Some("about".into()),
        1.0,
    )];
    let replacement_id = store.store_memory(&replacement).await.unwrap();

    assert_eq!(replacement_id, original_id);
    assert!(
        store
            .get_memory(&MemoryId("memory-replacement".into()))
            .await
            .unwrap()
            .is_none()
    );

    let loaded = store.get_memory(&original_id).await.unwrap().unwrap();
    assert_eq!(
        loaded.content,
        "Sam prefers concise deployment status updates"
    );
    assert_eq!(loaded.importance, 0.95);
    assert_eq!(loaded.subjects.len(), 1);
    assert_eq!(loaded.subjects[0].subject_id, "profile-sam-work");

    let old_profile_results = store
        .recall(
            &RecallQuery::by_text("deployment", 10).with_profile(ProfileId("profile-sam".into())),
        )
        .await
        .unwrap();
    assert!(old_profile_results.is_empty());

    let new_profile_results = store
        .recall(
            &RecallQuery::by_text("deployment", 10)
                .with_profile(ProfileId("profile-sam-work".into())),
        )
        .await
        .unwrap();
    assert_eq!(new_profile_results.len(), 1);
    assert_eq!(new_profile_results[0].id, original_id);

    let conn = store.lock().unwrap();
    let mutation_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM memory_mutations WHERE memory_id = ?1",
            params![original_id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mutation_count, 2);
}
#[tokio::test]
async fn memory_get_loads_embedding() {
    let store = test_store();
    let embedding = vec![0.1; 1024];
    store
        .store_memory(&sample_memory("m1", "test", embedding.clone()))
        .await
        .unwrap();

    let loaded = store
        .get_memory(&MemoryId("m1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.embedding.unwrap(), embedding);
}

#[tokio::test]
async fn memory_store_skips_incompatible_embedding_dimensions() {
    let store = test_store();
    let id = store
        .store_memory(&sample_memory("m1", "test", vec![0.1, 0.2, 0.3]))
        .await
        .unwrap();

    let loaded = store.get_memory(&id).await.unwrap().unwrap();
    assert_eq!(loaded.content, "test");
    assert!(loaded.embedding.is_none());
    assert!(loaded.embedding_model.is_none());
}

#[tokio::test]
async fn memory_update_skips_incompatible_embedding_dimensions() {
    let store = test_store();
    let embedding = vec![0.1; 1024];
    store
        .store_memory(&sample_memory("m1", "test", embedding))
        .await
        .unwrap();

    store
        .update_memory(
            &MemoryId("m1".into()),
            &MemoryUpdate {
                content: Some("updated".into()),
                embedding_model: Some("embed-1024".into()),
                embedding: Some(vec![0.1, 0.2, 0.3]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let loaded = store
        .get_memory(&MemoryId("m1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.content, "updated");
    assert!(loaded.embedding.is_none());
    assert!(loaded.embedding_model.is_none());
}
