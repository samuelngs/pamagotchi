use super::*;

#[tokio::test]
async fn memory_subjects_association() {
    let store = test_store();
    let mem = Memory {
        id: MemoryId("m1".into()),
        kind: MemoryKind::Episodic,
        content: "Alice told me Bob got a new job".into(),
        source: MemorySource::Conversation {
            conversation_id: ConversationId("c1".into()),
            identity_id: None,
            profile_id: Some(ProfileId("profile-alice".into())),
            person_id: Some(PersonId("alice".into())),
            message_id: None,
        },
        importance: 0.7,
        sensitivity: 0.5,
        emotional_valence: 0.3,
        created_at: 1000,
        accessed_at: 1000,
        access_count: 0,
        tags: vec![],
        subjects: vec![
            MemorySubject::profile(
                ProfileId("profile-alice".into()),
                Some("speaker".into()),
                1.0,
            ),
            MemorySubject::person(PersonId("bob".into()), Some("mentioned".into()), 0.8),
        ],
        embedding: None,
        ..Memory::default()
    };
    store.store_memory(&mem).await.unwrap();

    let loaded = store
        .get_memory(&MemoryId("m1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.subjects.len(), 2);

    let results = store
        .recall(&RecallQuery::by_text("Bob", 10).with_person(PersonId("bob".into())))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    let results = store
        .recall(&RecallQuery::by_text("Bob", 10).with_person(PersonId("charlie".into())))
        .await
        .unwrap();
    assert_eq!(results.len(), 0);
}
#[tokio::test]
async fn memory_subjects_can_be_rewritten_without_legacy_person_links() {
    let store = test_store();
    let mut mem = sample_memory(
        "promotable",
        "Sam prefers concise updates",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    mem.subjects = vec![MemorySubject::profile(
        ProfileId("profile-sam".into()),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&mem).await.unwrap();

    store
        .update_memory(
            &mem.id,
            &MemoryUpdate {
                content: None,
                importance: None,
                sensitivity: None,
                emotional_valence: None,
                tags: None,
                subjects: Some(vec![MemorySubject::person(
                    PersonId("sam".into()),
                    Some("about".into()),
                    1.0,
                )]),
                embedding: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let by_profile = store
        .recall(&RecallQuery::by_text("concise", 10).with_profile(ProfileId("profile-sam".into())))
        .await
        .unwrap();
    assert_eq!(by_profile.len(), 0);

    let by_person = store
        .recall(&RecallQuery::by_text("concise", 10).with_person(PersonId("sam".into())))
        .await
        .unwrap();
    assert_eq!(by_person.len(), 1);
}
