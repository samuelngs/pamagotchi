use super::*;

#[tokio::test]
async fn current_sender_person_is_not_overwritten_by_channel_or_group_context() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let now = chrono::Utc::now().timestamp();
    let conversation = ConversationId("discord:channel-identity".into());
    let group = GroupId("discord:guild-identity".into());
    let alice_person = PersonId("person-alice-current".into());
    let bob_person = PersonId("person-bob-stale-channel".into());
    let alice_profile = ProfileId("profile-alice-current".into());
    let bob_profile = ProfileId("profile-bob-stale-channel".into());
    let alice_identity = IdentityId("identity-alice-current".into());
    let bob_identity = IdentityId("identity-bob-stale-channel".into());

    for (person, name, summary) in [
        (
            alice_person.clone(),
            "Alice Current",
            "Alice is the current sender.",
        ),
        (
            bob_person.clone(),
            "Bob Channel",
            "Bob is stale conversation context only.",
        ),
        (
            PersonId("person-eve-group-member".into()),
            "Eve Group",
            "Eve is only a group member.",
        ),
    ] {
        store
            .add_person(&Person {
                id: person,
                name: Some(name.into()),
                summary: Some(summary.into()),
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
    }

    store
        .add_group(&Group {
            id: group.clone(),
            name: "Identity Guild".into(),
            gateway_id: "discord".into(),
            external_id: "guild-identity".into(),
            context: GroupContext::Work,
            members: vec![
                bob_person.clone(),
                PersonId("person-eve-group-member".into()),
            ],
        })
        .await
        .unwrap();

    for (identity, profile, person, external_id, display_name) in [
        (
            alice_identity.clone(),
            alice_profile.clone(),
            alice_person.clone(),
            "alice-author",
            "Alice Profile",
        ),
        (
            bob_identity.clone(),
            bob_profile.clone(),
            bob_person.clone(),
            "bob-author",
            "Bob Profile",
        ),
    ] {
        store
            .add_identity(&Identity {
                id: identity.clone(),
                gateway_id: "discord".into(),
                external_id: external_id.into(),
                display_name: Some(display_name.into()),
                metadata: None,
                created_at: now,
                last_seen_at: now,
            })
            .await
            .unwrap();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some(display_name.into()),
                summary: Some(format!("{display_name} summary")),
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .link_identity_to_profile(&identity, &profile, 1.0, None)
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
    }

    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now - 60,
                role: MessageRole::User,
                content: "stale channel context".into(),
                identity: Some(bob_identity.clone()),
                profile: Some(bob_profile.clone()),
                person: Some(bob_person.clone()),
                source_gateway_id: Some("discord".into()),
                source_message_id: Some("bob-old-msg".into()),
                sender_external_id: Some("bob-author".into()),
                reply_external_id: Some("channel-identity".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    let current = protocol::InboundMessage {
        message_id: "alice-current-msg".into(),
        gateway_id: "discord".into(),
        sender: Some(protocol::ObservedSender::primary(
            "discord",
            "alice-author",
            Some("Alice Current".into()),
            "test",
        )),
        channel: protocol::ChannelKey::new(
            "discord",
            "channel-identity",
            protocol::ChannelKind::PublicChannel,
        ),
        conversation: conversation.clone(),
        identity: Some(alice_identity.clone()),
        profile: Some(alice_profile.clone()),
        person: Some(alice_person.clone()),
        content: "this is Alice now".into(),
        attachments: vec![],
        timestamp: now,
        metadata: serde_json::json!({ "group_id": group.0 }),
    };

    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let mut actor = ActorState::new(CoreTraits::default());
    actor.set_relationship_config(&alice_person, Some(RelationshipStanding::Default));
    actor.set_relationship_config(&bob_person, Some(RelationshipStanding::Default));
    let shared = Arc::new(SharedState {
        actor: RwLock::new(actor),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state = StateHandle::new(shared, delta_tx);
    let ctx = SessionContext {
        action_id: ActionId("identity-context-test".into()),
        kind: SessionKind::Action(ActionKind::Respond),
        messages: vec![current],
        conversation: Some(conversation.clone()),
        relationship_standing: RelationshipStanding::Default,
        style_directive: None,
        cancelled_note: None,
        concurrent_summaries: vec![],
        state: state.clone(),
        store: store_dyn.clone(),
        media_store: None,
        router: Arc::new(test_router()),
        endpoints: vec![],
        reasoning: Reasoning::Basic,
        inject_rx,
        progress: Arc::new(RwLock::new(RunningState::new())),
        max_turns: 1,
        max_action_attempts: 1,
        escalate_after: 1,
        gateway: Arc::new(GatewayRouter::new()),
        typing: Arc::new(RwLock::new(Default::default())),
        metrics: Arc::new(crate::core::ActorMetrics::default()),
        session_start: std::time::Instant::now(),
    };

    let action_prompt = build_system_prompt(
        &state,
        &store_dyn,
        &ctx.kind,
        &ctx.messages,
        Some(&conversation),
        &ctx,
        &RelationshipStanding::Default,
    )
    .await
    .unwrap();
    let current_person = section_between(
        &action_prompt,
        "## Current verified person",
        "## Current conversation",
    );
    assert!(current_person.contains("- id: person-alice-current"));
    assert!(current_person.contains("- display name: Alice Current"));
    assert!(current_person.contains("- summary: Alice is the current sender."));
    assert!(!current_person.contains("person-bob-stale-channel"));
    assert!(!current_person.contains("Bob Channel"));
    assert!(!current_person.contains("Eve Group"));
    let current_channel = section_between(&action_prompt, "## Current channel", "## Current group");
    assert!(current_channel.contains("- external id: channel-identity"));
    assert!(current_channel.contains("- kind: public_channel"));
    assert!(current_channel.contains("not the sender identity or person"));
    assert!(!current_channel.contains("person-bob-stale-channel"));
    assert!(!current_channel.contains("Bob Channel"));
    let current_group =
        section_between(&action_prompt, "## Current group", "## Timing and delivery");
    assert!(current_group.contains("person-bob-stale-channel (Bob Channel)"));
    assert!(current_group.contains("person-eve-group-member (Eve Group)"));

    let mind_kind = SessionKind::Mind;
    let mind_prompt = build_system_prompt(
        &state,
        &store_dyn,
        &mind_kind,
        &ctx.messages,
        Some(&conversation),
        &ctx,
        &RelationshipStanding::Default,
    )
    .await
    .unwrap();
    let mind_person = section_between(
        &mind_prompt,
        "## Person [person-alice-current]",
        "## Current conversation",
    );
    assert!(mind_person.contains("Alice Current, Relationship standing: default"));
    assert!(mind_person.contains("Alice is the current sender."));
    assert!(!mind_person.contains("person-bob-stale-channel"));
    assert!(!mind_person.contains("Bob Channel"));
    assert!(!mind_person.contains("Eve Group"));
    let mind_channel = section_between(&mind_prompt, "## Current channel", "## Current group");
    assert!(mind_channel.contains("- external id: channel-identity"));
    assert!(mind_channel.contains("- kind: public_channel"));
    assert!(mind_channel.contains("not the sender identity or person"));
    assert!(!mind_channel.contains("person-bob-stale-channel"));
    assert!(!mind_channel.contains("Bob Channel"));
}

fn section_between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let start_index = text.find(start).expect("prompt section start exists");
    let after_start = &text[start_index..];
    let end_index = after_start
        .find(end)
        .expect("prompt section end exists after start");
    &after_start[..end_index]
}
