use super::*;

#[tokio::test]
async fn memory_type_recall_filters_before_limiting() {
    let store = test_store();
    let profile = ProfileId("profile-target".into());
    for idx in 0..60 {
        let mut memory = sample_memory(
            &format!("recent-fact-{idx}"),
            "recent generic observation",
            vec![0.1, 0.2, 0.3, 0.4],
        );
        memory.created_at = 20_000 + idx;
        memory.memory_type = MemoryType::Fact;
        memory.subjects = vec![MemorySubject::profile(
            profile.clone(),
            Some("about".into()),
            1.0,
        )];
        store.store_memory(&memory).await.unwrap();
    }

    let mut boundary = sample_memory(
        "older-boundary",
        "Do not send deployment reminders after 8pm.",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    boundary.created_at = 1000;
    boundary.memory_type = MemoryType::Boundary;
    boundary.subjects = vec![MemorySubject::profile(
        profile.clone(),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&boundary).await.unwrap();

    let results = store
        .recall(
            &RecallQuery::by_text("", 1)
                .with_profile(profile)
                .with_memory_type(MemoryType::Boundary),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "older-boundary");
}
#[tokio::test]
async fn recall_filters() {
    let store = test_store();
    store
        .store_memory(&Memory {
            id: MemoryId("m1".into()),
            kind: MemoryKind::Episodic,
            content: "episodic thing".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![],
            embedding: None,
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("m2".into()),
            kind: MemoryKind::Semantic,
            content: "semantic fact".into(),
            source: MemorySource::Reflection,
            importance: 0.3,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![],
            embedding: None,
            ..Memory::default()
        })
        .await
        .unwrap();

    let results = store
        .recall(&RecallQuery::by_text("thing", 10).with_kind(MemoryKind::Episodic))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");

    let results = store
        .recall(&RecallQuery::by_text("", 10).with_min_importance(0.5))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.0, "m1");
}
#[tokio::test]
async fn recall_filters_memories_due_for_review() {
    let store = test_store();
    for (id, next_review_at, privacy_category) in [
        ("due-public", Some(900), PrivacyCategory::Personal),
        ("future-public", Some(1200), PrivacyCategory::Personal),
        ("due-secret", Some(900), PrivacyCategory::Secret),
        ("not-scheduled", None, PrivacyCategory::Personal),
    ] {
        store
            .store_memory(&Memory {
                id: MemoryId(id.into()),
                kind: MemoryKind::Semantic,
                content: format!("{id} memory"),
                source: MemorySource::Reflection,
                importance: 0.8,
                confidence: 0.8,
                sensitivity: if matches!(privacy_category, PrivacyCategory::Secret) {
                    0.95
                } else {
                    0.0
                },
                privacy_category,
                next_review_at,
                created_at: 1000,
                accessed_at: 1000,
                access_count: 0,
                ..Memory::default()
            })
            .await
            .unwrap();
    }

    let results = store
        .recall(&RecallQuery::by_text("", 10).with_next_review_due(1000))
        .await
        .unwrap();
    assert_eq!(
        results
            .iter()
            .map(|memory| memory.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["due-public"]
    );

    let results = store
        .recall(
            &RecallQuery::by_text("", 10)
                .with_next_review_due(1000)
                .include_sensitive(),
        )
        .await
        .unwrap();
    let ids = results
        .iter()
        .map(|memory| memory.id.0.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"due-public"));
    assert!(ids.contains(&"due-secret"));
    assert!(!ids.contains(&"future-public"));
    assert!(!ids.contains(&"not-scheduled"));
}
#[tokio::test]
async fn actor_subject_recall_does_not_return_profile_identity_memories() {
    let store = test_store();

    store
        .store_memory(&Memory {
            id: MemoryId("profile-name".into()),
            kind: MemoryKind::Semantic,
            content: "Sam said my name is Sam".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-sam".into()),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("actor-name".into()),
            kind: MemoryKind::Semantic,
            content: "My name is Pamagotchi".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            subjects: vec![MemorySubject::actor(Some("self".into()), 1.0)],
            ..Memory::default()
        })
        .await
        .unwrap();

    let actor_memories = store
        .recall(&RecallQuery::by_text("my name", 10).with_actor_subject())
        .await
        .unwrap();

    assert_eq!(actor_memories.len(), 1);
    assert_eq!(actor_memories[0].id.0, "actor-name");
}
