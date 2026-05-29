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
        assert_eq!(
            actor.bonds[&person].relationship_standing,
            RelationshipStanding::Default
        );
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
        relationship_record.payload["relationship_standing"].as_str(),
        Some("default")
    );

    drop(mind);
    state_join.await.unwrap();
}
#[tokio::test]
async fn discord_channel_resolves_authors_as_distinct_profiles_in_one_conversation() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let gateway = GatewayId("discord".into());
    upsert_test_channel(
        store.as_ref(),
        &gateway,
        "channel-1",
        ChannelKind::GroupChat,
    )
    .await;

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
    let memberships = store
        .list_channel_memberships(&channel_id(&gateway, "channel-1"))
        .await
        .unwrap();
    assert_eq!(memberships.len(), 2);
    assert!(
        memberships
            .iter()
            .any(|m| Some(&m.profile) == alice.profile.as_ref())
    );
    assert!(
        memberships
            .iter()
            .any(|m| Some(&m.profile) == bob.profile.as_ref())
    );
    assert!(store.debug_groups(10).await.unwrap().is_empty());
}
#[tokio::test]
async fn whatsapp_group_sender_memories_are_profile_scoped() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let gateway = GatewayId("whatsapp".into());
    upsert_test_channel(
        store.as_ref(),
        &gateway,
        "family@g.us",
        ChannelKind::GroupChat,
    )
    .await;

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
    let memberships = store
        .list_channel_memberships(&channel_id(&gateway, "family@g.us"))
        .await
        .unwrap();
    assert_eq!(memberships.len(), 2);
    assert!(
        memberships
            .iter()
            .any(|m| Some(&m.profile) == alice.profile.as_ref())
    );
    assert!(
        memberships
            .iter()
            .any(|m| Some(&m.profile) == bob.profile.as_ref())
    );
    assert!(store.debug_groups(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn whatsapp_group_sender_alias_links_to_existing_dm_profile() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let gateway = GatewayId("whatsapp".into());
    upsert_test_channel(
        store.as_ref(),
        &gateway,
        "alice@s.whatsapp.net",
        ChannelKind::Direct,
    )
    .await;
    upsert_test_channel(
        store.as_ref(),
        &gateway,
        "family@g.us",
        ChannelKind::GroupChat,
    )
    .await;

    let mut dm = inbound(
        "whatsapp",
        "alice@s.whatsapp.net",
        "Alice",
        "alice@s.whatsapp.net",
        "whatsapp:alice@s.whatsapp.net",
        None,
        "wa-dm-1",
    );
    dm.metadata = whatsapp_sender_metadata(
        "alice@s.whatsapp.net",
        ChannelKind::Direct,
        "alice@s.whatsapp.net",
        vec![],
        "wa-dm-1",
        "Alice",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut dm).await;
    let dm_profile = dm.profile.clone().expect("dm sender profile");

    let mut group = inbound(
        "whatsapp",
        "alice@lid.whatsapp.net",
        "Alice",
        "family@g.us",
        "whatsapp:family@g.us",
        Some("family@g.us"),
        "wa-group-1",
    );
    group.metadata = whatsapp_sender_metadata(
        "family@g.us",
        ChannelKind::GroupChat,
        "alice@lid.whatsapp.net",
        vec!["alice@s.whatsapp.net"],
        "wa-group-1",
        "Alice",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut group).await;

    assert_eq!(group.profile.as_ref(), Some(&dm_profile));
    let primary = store
        .resolve_identity("whatsapp", "alice@lid.whatsapp.net")
        .await
        .unwrap()
        .unwrap();
    let alias = store
        .resolve_identity("whatsapp", "alice@s.whatsapp.net")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(primary.profile.id, dm_profile);
    assert_eq!(alias.profile.id, dm_profile);

    let group_channel = channel_id(&gateway, "family@g.us");
    let memberships = store
        .list_channel_memberships(&group_channel)
        .await
        .unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].profile, primary.profile.id);
}

#[tokio::test]
async fn conflicting_whatsapp_sender_aliases_record_conflict_without_merging_profiles() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let gateway = GatewayId("whatsapp".into());
    upsert_test_channel(
        store.as_ref(),
        &gateway,
        "family@g.us",
        ChannelKind::GroupChat,
    )
    .await;

    let pn_identity = Identity {
        id: IdentityId("identity-pn".into()),
        gateway_id: "whatsapp".into(),
        external_id: "alice@s.whatsapp.net".into(),
        display_name: Some("Alice PN".into()),
        metadata: None,
        created_at: 900,
        last_seen_at: 900,
    };
    let lid_identity = Identity {
        id: IdentityId("identity-lid".into()),
        gateway_id: "whatsapp".into(),
        external_id: "alice@lid.whatsapp.net".into(),
        display_name: Some("Alice LID".into()),
        metadata: None,
        created_at: 900,
        last_seen_at: 900,
    };
    let pn_profile = Profile {
        id: ProfileId("profile-pn".into()),
        display_name: Some("Alice PN".into()),
        summary: None,
        comm_style: None,
        first_seen: 900,
        last_seen: 900,
        created_at: 900,
        updated_at: 900,
    };
    let lid_profile = Profile {
        id: ProfileId("profile-lid".into()),
        display_name: Some("Alice LID".into()),
        summary: None,
        comm_style: None,
        first_seen: 900,
        last_seen: 900,
        created_at: 900,
        updated_at: 900,
    };
    let pn_person = Person {
        id: PersonId("person-pn".into()),
        name: Some("Alice PN".into()),
        summary: None,
        comm_style: None,
        first_seen: 900,
        last_seen: 900,
    };
    let lid_person = Person {
        id: PersonId("person-lid".into()),
        name: Some("Alice LID".into()),
        summary: None,
        comm_style: None,
        first_seen: 900,
        last_seen: 900,
    };
    store.add_identity(&pn_identity).await.unwrap();
    store.add_identity(&lid_identity).await.unwrap();
    store.add_profile(&pn_profile).await.unwrap();
    store.add_profile(&lid_profile).await.unwrap();
    store.add_person(&pn_person).await.unwrap();
    store.add_person(&lid_person).await.unwrap();
    store
        .link_identity_to_profile(&pn_identity.id, &pn_profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .link_identity_to_profile(&lid_identity.id, &lid_profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &pn_profile.id,
            &pn_person.id,
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &lid_profile.id,
            &lid_person.id,
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();

    let mut msg = inbound(
        "whatsapp",
        "alice@lid.whatsapp.net",
        "Alice",
        "family@g.us",
        "whatsapp:family@g.us",
        Some("family@g.us"),
        "wa-conflict-1",
    );
    msg.metadata = whatsapp_sender_metadata(
        "family@g.us",
        ChannelKind::GroupChat,
        "alice@lid.whatsapp.net",
        vec!["alice@s.whatsapp.net"],
        "wa-conflict-1",
        "Alice",
    );
    ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

    assert_eq!(msg.identity.as_ref(), Some(&lid_identity.id));
    assert_eq!(msg.profile.as_ref(), Some(&lid_profile.id));
    assert_eq!(msg.person.as_ref(), Some(&lid_person.id));
    let still_pn = store
        .resolve_identity("whatsapp", "alice@s.whatsapp.net")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still_pn.profile.id, pn_profile.id);

    let conflicts = store.identity_conflicts(10).await.unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].status, "open");
    assert_eq!(
        conflicts[0].primary_identity.as_ref(),
        Some(&lid_identity.id)
    );
    assert!(conflicts[0].profiles.contains(&pn_profile.id));
    assert!(conflicts[0].profiles.contains(&lid_profile.id));
    assert_eq!(conflicts[0].identities.len(), 2);
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

async fn upsert_test_channel(
    store: &SqliteStore,
    gateway: &GatewayId,
    external_id: &str,
    kind: ChannelKind,
) {
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway.clone(),
            kind: gateway.0.clone(),
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    store
        .upsert_channel(&ChannelRecord {
            id: channel_id(gateway, external_id),
            gateway: gateway.clone(),
            external_id: external_id.into(),
            kind,
            space: None,
            parent: None,
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: 1000,
            updated_at: 1000,
            last_seen_at: 1000,
        })
        .await
        .unwrap();
}

fn whatsapp_sender_metadata(
    channel_external_id: &str,
    channel_kind: ChannelKind,
    primary_external_id: &str,
    aliases: Vec<&str>,
    message_id: &str,
    display_name: &str,
) -> serde_json::Value {
    let gateway = GatewayId("whatsapp".into());
    let envelope = InboundEnvelope {
        gateway_id: gateway.clone(),
        platform_message_id: message_id.into(),
        channel: ChannelKey {
            gateway_id: gateway.clone(),
            external_id: channel_external_id.into(),
            kind: channel_kind,
            display_name: None,
            space: None,
            parent: None,
            metadata: serde_json::json!({}),
        },
        sender: Some(ObservedSender {
            primary: observed_whatsapp_key(&gateway, primary_external_id, "primary_sender"),
            aliases: aliases
                .into_iter()
                .map(|alias| observed_whatsapp_key(&gateway, alias, "sender_alt"))
                .collect(),
            display_name: Some(display_name.into()),
            metadata: serde_json::json!({}),
        }),
        content: "hello".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: serde_json::json!({}),
    };
    serde_json::json!({ "normalized_envelope": envelope })
}

fn observed_whatsapp_key(
    gateway: &GatewayId,
    external_id: &str,
    source: &str,
) -> ObservedIdentityKey {
    ObservedIdentityKey {
        gateway_id: gateway.clone(),
        external_id: external_id.into(),
        kind: Some("whatsapp_user".into()),
        confidence: 1.0,
        source: source.into(),
    }
}
