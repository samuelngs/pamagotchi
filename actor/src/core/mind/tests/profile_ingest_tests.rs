use super::*;

#[tokio::test]
async fn first_relay_contact_starts_as_default_adoption_candidate() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (mind, state_join) = test_mind_with_state_task(store.clone());
    let mut msg = inbound(
        "relay",
        "sam-local",
        "Sam",
        "sam-local",
        "relay:sam-local",
        None,
        "relay-chosen_human-msg-1",
    );

    ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

    let person = msg
        .person
        .clone()
        .expect("relay contact resolves to a person");
    {
        let actor = mind.state.read_state();
        assert_eq!(actor.bonds[&person].authority, Authority::Default);
    }
    let records = store.state_journal_after(None, 10).await.unwrap();
    let relationship_record = records
        .iter()
        .find(|record| record.kind == "relationship_config")
        .expect("relationship config journal record");
    assert_eq!(
        relationship_record.payload["person_id"].as_str(),
        Some(person.0.as_str())
    );
    assert_eq!(
        relationship_record.payload["authority"].as_str(),
        Some("default")
    );

    drop(mind);
    state_join.await.unwrap();
}
#[tokio::test]
async fn discord_channel_resolves_authors_as_distinct_profiles_in_one_conversation() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());

    let mut alice = inbound(
        "discord",
        "author-a",
        "Alice",
        "channel-1",
        "discord:channel-1",
        Some("discord:guild-1"),
        "msg-a",
    );
    let mut bob = inbound(
        "discord",
        "author-b",
        "Bob",
        "channel-1",
        "discord:channel-1",
        Some("discord:guild-1"),
        "msg-b",
    );

    ingest::resolve_person(&mind.state, &mind.store, &mut alice).await;
    ingest::resolve_person(&mind.state, &mind.store, &mut bob).await;
    append_inbound(store.as_ref(), &alice).await;
    append_inbound(store.as_ref(), &bob).await;

    assert_ne!(alice.identity, bob.identity);
    assert_ne!(alice.profile, bob.profile);
    assert_eq!(alice.conversation, bob.conversation);
    assert!(
        store
            .resolve_identity("discord", "channel-1")
            .await
            .unwrap()
            .is_none()
    );

    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(conversations.len(), 1);
    assert_eq!(
        conversations[0].id,
        ConversationId("discord:channel-1".into())
    );
    assert_eq!(
        conversations[0].group.as_ref(),
        Some(&GroupId("discord:guild-1".into()))
    );
    let group = store
        .get_group(&GroupId("discord:guild-1".into()))
        .await
        .unwrap()
        .unwrap();
    assert!(group.members.contains(alice.person.as_ref().unwrap()));
    assert!(group.members.contains(bob.person.as_ref().unwrap()));
}
#[tokio::test]
async fn whatsapp_group_sender_memories_are_profile_scoped() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());

    let mut alice = inbound(
        "whatsapp",
        "alice@s.whatsapp.net",
        "Alice",
        "family@g.us",
        "whatsapp:family@g.us",
        Some("family@g.us"),
        "wa-msg-a",
    );
    let mut bob = inbound(
        "whatsapp",
        "bob@s.whatsapp.net",
        "Bob",
        "family@g.us",
        "whatsapp:family@g.us",
        Some("family@g.us"),
        "wa-msg-b",
    );

    ingest::resolve_person(&mind.state, &mind.store, &mut alice).await;
    ingest::resolve_person(&mind.state, &mind.store, &mut bob).await;
    append_inbound(store.as_ref(), &alice).await;
    append_inbound(store.as_ref(), &bob).await;

    let alice_profile = alice.profile.clone().unwrap();
    let bob_profile = bob.profile.clone().unwrap();
    assert_ne!(alice_profile, bob_profile);

    store
        .store_memory(&Memory {
            id: MemoryId("memory-alice".into()),
            kind: MemoryKind::Semantic,
            content: "Alice prefers concise deployment updates.".into(),
            source: MemorySource::Conversation {
                conversation_id: alice.conversation.clone(),
                identity_id: alice.identity.clone(),
                profile_id: Some(alice_profile.clone()),
                person_id: alice.person.clone(),
                message_id: Some(alice.message_id.clone()),
            },
            importance: 0.8,
            sensitivity: 0.0,
            emotional_valence: 0.0,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            subjects: vec![MemorySubject::profile(
                alice_profile.clone(),
                Some("about".into()),
                1.0,
            )],
            embedding: None,
            ..Memory::default()
        })
        .await
        .unwrap();

    let alice_memories = store
        .recall(&RecallQuery::by_text("deployment", 10).with_profile(alice_profile))
        .await
        .unwrap();
    let bob_memories = store
        .recall(&RecallQuery::by_text("deployment", 10).with_profile(bob_profile))
        .await
        .unwrap();

    assert_eq!(alice_memories.len(), 1);
    assert!(bob_memories.is_empty());

    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(conversations.len(), 1);
    assert_eq!(
        conversations[0].group.as_ref(),
        Some(&GroupId("family@g.us".into()))
    );
    let group = store
        .get_group(&GroupId("family@g.us".into()))
        .await
        .unwrap()
        .unwrap();
    assert!(group.members.contains(alice.person.as_ref().unwrap()));
    assert!(group.members.contains(bob.person.as_ref().unwrap()));
}
#[tokio::test]
async fn existing_gateway_identity_refreshes_observed_display_name() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let identity = Identity {
        id: IdentityId("identity-local".into()),
        gateway_id: "relay".into(),
        external_id: "local".into(),
        display_name: Some("Old Sam".into()),
        metadata: None,
        created_at: 900,
        last_seen_at: 900,
    };
    let profile = Profile {
        id: ProfileId("profile-local".into()),
        display_name: None,
        summary: None,
        comm_style: None,
        first_seen: 900,
        last_seen: 900,
        created_at: 900,
        updated_at: 900,
    };
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();

    let mut msg = inbound(
        "relay",
        "local",
        "New Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

    let refreshed_identity = store.get_identity(&identity.id).await.unwrap().unwrap();
    let refreshed_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
    assert_eq!(refreshed_identity.display_name.as_deref(), Some("New Sam"));
    assert_eq!(refreshed_profile.display_name.as_deref(), Some("New Sam"));
    let observations = store
        .display_name_observations(&identity.id, 10)
        .await
        .unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].display_name, "New Sam");
    assert_eq!(
        observations[0].source_message_id.as_deref(),
        Some("local-msg-1")
    );
    assert_eq!(observations[0].profile.as_ref(), Some(&profile.id));

    store
        .update_profile(&profile.id, Some("Preferred Sam"), None)
        .await
        .unwrap();
    let mut msg = inbound(
        "relay",
        "local",
        "Newest Sam",
        "local",
        "relay:local",
        None,
        "local-msg-2",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

    let refreshed_identity = store.get_identity(&identity.id).await.unwrap().unwrap();
    let preserved_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
    assert_eq!(
        refreshed_identity.display_name.as_deref(),
        Some("Newest Sam")
    );
    assert_eq!(
        preserved_profile.display_name.as_deref(),
        Some("Preferred Sam")
    );
    let observations = store
        .display_name_observations(&identity.id, 10)
        .await
        .unwrap();
    assert_eq!(
        observations
            .iter()
            .map(|observation| observation.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["New Sam", "Newest Sam"]
    );

    let mut duplicate_msg = inbound(
        "relay",
        "local",
        "Newest Sam",
        "local",
        "relay:local",
        None,
        "local-msg-2",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut duplicate_msg).await;
    let observations = store
        .display_name_observations(&identity.id, 10)
        .await
        .unwrap();
    assert_eq!(observations.len(), 2);
}
#[tokio::test]
async fn existing_gateway_identity_refreshes_auto_profile_display_name() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let identity = Identity {
        id: IdentityId("identity-auto-name".into()),
        gateway_id: "relay".into(),
        external_id: "local".into(),
        display_name: Some("Old Sam".into()),
        metadata: None,
        created_at: 900,
        last_seen_at: 900,
    };
    let profile = Profile {
        id: ProfileId("profile-auto-name".into()),
        display_name: Some("Old Sam".into()),
        summary: None,
        comm_style: None,
        first_seen: 900,
        last_seen: 900,
        created_at: 900,
        updated_at: 900,
    };
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();

    let mut msg = inbound(
        "relay",
        "local",
        "New Sam",
        "local",
        "relay:local",
        None,
        "local-msg-auto-name",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

    let refreshed_identity = store.get_identity(&identity.id).await.unwrap().unwrap();
    let refreshed_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
    assert_eq!(refreshed_identity.display_name.as_deref(), Some("New Sam"));
    assert_eq!(refreshed_profile.display_name.as_deref(), Some("New Sam"));
}
