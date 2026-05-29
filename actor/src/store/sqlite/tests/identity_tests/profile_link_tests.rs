use super::*;

#[tokio::test]
async fn profile_attach_reconnects_identity_without_deleting_person() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice"))
        .await
        .unwrap();
    store
        .add_person(&sample_person("p2", "Alice Alt"))
        .await
        .unwrap();

    let identity = sample_identity("i2", "telegram", "tg-alice", "alice_t");
    let profile = sample_profile("profile-p2", "alice_t");
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            &PersonId("p2".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let conv = ConversationId("c1".into());
    store
        .append_message(
            &conv,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "from alt account".into(),
                identity: Some(identity.id.clone()),
                profile: Some(profile.id.clone()),
                person: Some(PersonId("p2".into())),
                source_gateway_id: Some("telegram".into()),
                source_message_id: Some("tg-msg-1".into()),
                sender_external_id: Some("tg-alice".into()),
                reply_external_id: Some("tg-chat".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    store
        .attach_profile_to_person(
            &profile.id,
            &PersonId("p1".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let resolved = store
        .resolve_identity("telegram", "tg-alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.person.unwrap().id.0, "p1");

    assert!(
        store
            .get_person(&PersonId("p2".into()))
            .await
            .unwrap()
            .is_some()
    );
}
#[tokio::test]
async fn same_display_name_does_not_share_profile_memories() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    store.add_person(&sample_person("p2", "Sam")).await.unwrap();

    let identity_a = sample_identity("i1", "discord", "sam-a", "Sam");
    let profile_a = sample_profile("profile-a", "Sam");
    store.add_identity(&identity_a).await.unwrap();
    store.add_profile(&profile_a).await.unwrap();
    store
        .link_identity_to_profile(&identity_a.id, &profile_a.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile_a.id,
            &PersonId("p1".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let identity_b = sample_identity("i2", "discord", "sam-b", "Sam");
    let profile_b = sample_profile("profile-b", "Sam");
    store.add_identity(&identity_b).await.unwrap();
    store.add_profile(&profile_b).await.unwrap();
    store
        .link_identity_to_profile(&identity_b.id, &profile_b.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile_b.id,
            &PersonId("p2".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    store
        .store_memory(&Memory {
            id: MemoryId("sam-a-city".into()),
            kind: MemoryKind::Semantic,
            content: "Sam said they are from Edmonton".into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("c1".into()),
                identity_id: Some(identity_a.id.clone()),
                profile_id: Some(profile_a.id.clone()),
                person_id: Some(PersonId("p1".into())),
                message_id: None,
            },
            importance: 0.8,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![MemorySubject::profile(
                profile_a.id.clone(),
                Some("about".into()),
                1.0,
            )],
            embedding: None,
            ..Memory::default()
        })
        .await
        .unwrap();

    let current_profile_results = store
        .recall(&RecallQuery::by_text("Edmonton", 10).with_profile(profile_a.id))
        .await
        .unwrap();
    assert_eq!(current_profile_results.len(), 1);

    let same_name_other_profile_results = store
        .recall(&RecallQuery::by_text("Edmonton", 10).with_profile(profile_b.id))
        .await
        .unwrap();
    assert_eq!(same_name_other_profile_results.len(), 0);
}
#[tokio::test]
async fn profile_comm_style_is_stored_on_profile_not_person() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    let profile = sample_profile("profile-sam", "Sam");
    store.add_profile(&profile).await.unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            &PersonId("p1".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    store
        .update_profile_comm_style(&profile.id, "Prefers concise replies")
        .await
        .unwrap();

    let loaded_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
    assert_eq!(
        loaded_profile.comm_style.as_deref(),
        Some("Prefers concise replies")
    );

    let loaded_person = store
        .get_person(&PersonId("p1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded_person.comm_style, None);
}
#[tokio::test]
async fn detach_profile_removes_person_context_without_rewriting_memories() {
    let store = test_store();
    store
        .add_person(&sample_person("p1", "Alice"))
        .await
        .unwrap();

    let identity = sample_identity("i1", "telegram", "alice-alt", "Alice");
    let profile = sample_profile("profile-alice-alt", "Alice");
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            &PersonId("p1".into()),
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    store
        .store_memory(&Memory {
            id: MemoryId("profile-memory".into()),
            kind: MemoryKind::Semantic,
            content: "Alice alt prefers short messages".into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("c1".into()),
                identity_id: Some(identity.id.clone()),
                profile_id: Some(profile.id.clone()),
                person_id: Some(PersonId("p1".into())),
                message_id: None,
            },
            importance: 0.8,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![MemorySubject::profile(
                profile.id.clone(),
                Some("about".into()),
                1.0,
            )],
            embedding: None,
            ..Memory::default()
        })
        .await
        .unwrap();

    assert!(
        store
            .get_person_for_profile(&profile.id)
            .await
            .unwrap()
            .is_some()
    );
    store
        .detach_profile_from_person(&profile.id, &PersonId("p1".into()), None)
        .await
        .unwrap();
    assert!(
        store
            .get_person_for_profile(&profile.id)
            .await
            .unwrap()
            .is_none()
    );

    let loaded = store
        .get_memory(&MemoryId("profile-memory".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.subjects[0].subject_id, profile.id.0);
}
