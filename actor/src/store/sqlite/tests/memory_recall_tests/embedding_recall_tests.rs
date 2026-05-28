use super::*;

#[tokio::test]
async fn memory_recall_by_embedding() {
    let store = test_store();
    store
        .store_memory(&sample_memory("m1", "first", vec![1.0, 0.0, 0.0, 0.0]))
        .await
        .unwrap();
    store
        .store_memory(&sample_memory("m2", "second", vec![0.0, 1.0, 0.0, 0.0]))
        .await
        .unwrap();

    let results = store
        .recall(&RecallQuery::by_embedding(vec![0.9, 0.1, 0.0, 0.0], 1))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");
}
#[tokio::test]
async fn scoped_embedding_recall_filters_before_vector_limit() {
    let store = test_store();
    let target_profile = ProfileId("profile-target".into());

    for idx in 0..220 {
        let mut memory = sample_memory(
            &format!("other-near-{idx}"),
            "unrelated deployment memory",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        memory.created_at = 10_000 + idx;
        memory.subjects = vec![MemorySubject::profile(
            ProfileId("profile-other".into()),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut scoped = sample_memory(
        "scoped-far",
        "target profile deployment memory",
        vec![0.0, 1.0, 0.0, 0.0],
    );
    scoped.subjects = vec![MemorySubject::profile(
        target_profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&scoped).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(vec![1.0, 0.0, 0.0, 0.0], 1).with_profile(target_profile),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "scoped-far");
}
#[tokio::test]
async fn scoped_embedding_recall_filters_sensitivity_after_vector_ranking() {
    let store = test_store();
    let target_profile = ProfileId("profile-target".into());

    for idx in 0..25 {
        let mut memory = sample_memory(
            &format!("sensitive-near-{idx}"),
            "sensitive deployment memory",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        memory.privacy_category = PrivacyCategory::Sensitive;
        memory.sensitivity = 0.8;
        memory.subjects = vec![MemorySubject::profile(
            target_profile.clone(),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut public = sample_memory(
        "public-far",
        "public target profile deployment memory",
        vec![0.0, 1.0, 0.0, 0.0],
    );
    public.subjects = vec![MemorySubject::profile(
        target_profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&public).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(vec![1.0, 0.0, 0.0, 0.0], 1).with_profile(target_profile),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "public-far");
}
#[tokio::test]
async fn embedding_recall_filters_memory_type_before_vector_limit() {
    let store = test_store();

    for idx in 0..220 {
        let mut memory = sample_memory(
            &format!("near-fact-{idx}"),
            "generic deployment fact",
            vec![1.0, 0.0, 0.0, 0.0],
        );
        memory.created_at = 10_000 + idx;
        memory.memory_type = MemoryType::Fact;
        store.store_memory(&memory).await.unwrap();
    }

    let mut boundary = sample_memory(
        "far-boundary",
        "Do not send deployment reminders after 8pm.",
        vec![0.0, 1.0, 0.0, 0.0],
    );
    boundary.memory_type = MemoryType::Boundary;
    store.store_memory(&boundary).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(vec![1.0, 0.0, 0.0, 0.0], 1)
                .with_memory_type(MemoryType::Boundary),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "far-boundary");
}
