use super::*;

#[tokio::test]
async fn memory_recall_by_embedding() {
    let store = test_store();
    store
        .store_memory(&sample_memory("m1", "first", basis_embedding(0)))
        .await
        .unwrap();
    store
        .store_memory(&sample_memory("m2", "second", basis_embedding(1)))
        .await
        .unwrap();

    let results = store
        .recall(&RecallQuery::by_embedding(query_embedding(0.9, 0.1), 1))
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
            basis_embedding(0),
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
        basis_embedding(1),
    );
    scoped.subjects = vec![MemorySubject::profile(
        target_profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&scoped).await.unwrap();

    let results = store
        .recall(&RecallQuery::by_embedding(basis_embedding(0), 1).with_profile(target_profile))
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
            basis_embedding(0),
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
        basis_embedding(1),
    );
    public.subjects = vec![MemorySubject::profile(
        target_profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&public).await.unwrap();

    let results = store
        .recall(&RecallQuery::by_embedding(basis_embedding(0), 1).with_profile(target_profile))
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
            basis_embedding(0),
        );
        memory.created_at = 10_000 + idx;
        memory.memory_type = MemoryType::Fact;
        store.store_memory(&memory).await.unwrap();
    }

    let mut boundary = sample_memory(
        "far-boundary",
        "Do not send deployment reminders after 8pm.",
        basis_embedding(1),
    );
    boundary.memory_type = MemoryType::Boundary;
    store.store_memory(&boundary).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_embedding(basis_embedding(0), 1)
                .with_memory_type(MemoryType::Boundary),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "far-boundary");
}

fn basis_embedding(index: usize) -> Vec<f32> {
    let mut embedding = vec![0.0; 1024];
    embedding[index] = 1.0;
    embedding
}

fn query_embedding(first: f32, second: f32) -> Vec<f32> {
    let mut embedding = vec![0.0; 1024];
    embedding[0] = first;
    embedding[1] = second;
    embedding
}
