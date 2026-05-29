use super::*;

#[tokio::test]
async fn group_directive_appears_after_first_group_inbound_is_persisted() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let person_id = PersonId("person-alice".into());
    let profile_id = ProfileId("profile-alice".into());
    let identity_id = IdentityId("identity-alice".into());
    let conversation = ConversationId("discord:channel-1".into());
    let group = GroupId("discord:guild-1".into());
    let now = chrono::Utc::now().timestamp();

    store
        .add_person(&Person {
            id: person_id.clone(),
            name: Some("Alice".into()),
            summary: Some("Alice coordinates deployment releases.".into()),
            comm_style: Some("Prefers practical release notes with direct status.".into()),
            first_seen: now,
            last_seen: now,
        })
        .await
        .unwrap();
    for (id, name) in [
        ("person-bob", "Bob"),
        ("person-carol", "Carol"),
        ("person-dave", "Dave"),
        ("person-eve", "Eve"),
    ] {
        store
            .add_person(&Person {
                id: PersonId(id.into()),
                name: Some(name.into()),
                summary: None,
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
            name: "Deploy Guild".into(),
            gateway_id: "discord".into(),
            external_id: "guild-1".into(),
            context: GroupContext::Work,
            members: vec![
                person_id.clone(),
                PersonId("person-bob".into()),
                PersonId("person-eve".into()),
            ],
        })
        .await
        .unwrap();
    store
        .add_profile(&Profile {
            id: profile_id.clone(),
            display_name: Some("Alice".into()),
            summary: Some("Alice's Discord profile tracks deployment coordination.".into()),
            comm_style: Some("On Discord, Alice prefers terse release status.".into()),
            first_seen: now,
            last_seen: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile_id,
            &person_id,
            PersonProfileStatus::Verified,
            0.95,
            Some(&serde_json::json!({"test": "profile prompt context"})),
        )
        .await
        .unwrap();
    store
        .upsert_relation(&SocialRelation {
            person_a: person_id.clone(),
            person_b: PersonId("person-bob".into()),
            relation: Relation::Coworker,
            direction: Relation::Coworker.default_direction(),
            confidence: 0.8,
            status: RelationStatus::Confirmed,
            evidence: Some(serde_json::json!({"message_id": "social-msg-1"})),
            source_kind: RelationSource::Stated,
            asserted_by: Some(person_id.clone()),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .upsert_relation(&SocialRelation {
            person_a: PersonId("person-carol".into()),
            person_b: person_id.clone(),
            relation: Relation::Friend,
            direction: Relation::Friend.default_direction(),
            confidence: 0.45,
            status: RelationStatus::Hypothesis,
            evidence: Some(serde_json::json!({"reason": "seen together in channel"})),
            source_kind: RelationSource::Inferred,
            asserted_by: None,
            created_at: now,
            updated_at: now - 1,
        })
        .await
        .unwrap();
    store
        .upsert_relation(&SocialRelation {
            person_a: person_id.clone(),
            person_b: PersonId("person-dave".into()),
            relation: Relation::Friend,
            direction: Relation::Friend.default_direction(),
            confidence: 0.9,
            status: RelationStatus::Denied,
            evidence: Some(serde_json::json!({"message_id": "social-msg-2"})),
            source_kind: RelationSource::Stated,
            asserted_by: Some(person_id.clone()),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .upsert_relation(&SocialRelation {
            person_a: PersonId("person-bob".into()),
            person_b: PersonId("person-dave".into()),
            relation: Relation::Friend,
            direction: Relation::Friend.default_direction(),
            confidence: 0.9,
            status: RelationStatus::Confirmed,
            evidence: Some(serde_json::json!({"message_id": "social-msg-3"})),
            source_kind: RelationSource::Stated,
            asserted_by: Some(PersonId("person-bob".into())),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .add_directive(&BehaviorDirective {
            id: "group-directive".into(),
            scope: DirectiveScope::Group(group.clone()),
            directive: "Use the group norm: keep deployment updates brief.".into(),
            set_by: person_id.clone(),
            priority: 10,
            active: true,
            created_at: now,
            expires_at: None,
        })
        .await
        .unwrap();

    let inbound = InboundMessage {
        message_id: "discord-msg-1".into(),
        gateway_id: "discord".into(),
        sender_external_id: "author-a".into(),
        sender_display_name: Some("Alice".into()),
        reply_external_id: "channel-1".into(),
        conversation: conversation.clone(),
        group: Some(group.clone()),
        identity: Some(identity_id.clone()),
        profile: Some(profile_id.clone()),
        person: Some(person_id.clone()),
        content: "deploy status?".into(),
        attachments: vec![],
        timestamp: now,
        metadata: serde_json::Value::Null,
    };
    store
        .append_message(
            &conversation,
            Some("discord"),
            Some(&group),
            &StoredMessage {
                timestamp: now - 60,
                role: MessageRole::User,
                content: "previous deploy thread".into(),
                identity: Some(identity_id.clone()),
                profile: Some(profile_id.clone()),
                person: Some(person_id.clone()),
                source_gateway_id: Some("discord".into()),
                source_message_id: Some("discord-msg-0".into()),
                sender_external_id: Some("author-a".into()),
                reply_external_id: Some("channel-1".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .append_message(
            &conversation,
            Some("discord"),
            Some(&group),
            &StoredMessage {
                timestamp: now,
                role: MessageRole::User,
                content: inbound.content.clone(),
                identity: Some(identity_id),
                profile: Some(profile_id.clone()),
                person: Some(person_id.clone()),
                source_gateway_id: Some("discord".into()),
                source_message_id: Some("discord-msg-1".into()),
                sender_external_id: Some("author-a".into()),
                reply_external_id: Some("channel-1".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .update_conversation_summary(
            &conversation,
            "Alice asked for concise deployment status in this channel.",
            &[String::from("discord-msg-1")],
        )
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("memory-current-profile".into()),
            kind: MemoryKind::Semantic,
            content: "Alice prefers brief deployment status updates.".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            confidence: 0.8,
            subjects: vec![MemorySubject::profile(
                profile_id.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("memory-current-boundary".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Boundary,
            content: "Do not mention surprise party details in shared channels.".into(),
            source: MemorySource::Reflection,
            importance: 0.85,
            confidence: 0.9,
            subjects: vec![MemorySubject::profile(
                profile_id.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    for idx in 0..45 {
        store
            .store_memory(&Memory {
                id: MemoryId(format!("memory-recent-generic-{idx}")),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Fact,
                content: format!("Recent generic observation {idx}."),
                source: MemorySource::Reflection,
                importance: 0.95,
                confidence: 0.95,
                created_at: now + idx,
                subjects: vec![MemorySubject::profile(
                    profile_id.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
    }
    store
        .store_memory(&Memory {
            id: MemoryId("memory-other-profile".into()),
            kind: MemoryKind::Semantic,
            content: "Other profile wants verbose deployment status updates.".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            confidence: 0.8,
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-other".into()),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("memory-sensitive-profile".into()),
            kind: MemoryKind::Semantic,
            content: "Alice has a secret deployment credential.".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            confidence: 0.8,
            sensitivity: 0.95,
            privacy_category: PrivacyCategory::Secret,
            subjects: vec![MemorySubject::profile(
                profile_id.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("memory-due-review".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Hypothesis,
            content: "Old launch checklist may be stale.".into(),
            source: MemorySource::Reflection,
            importance: 0.7,
            confidence: 0.6,
            next_review_at: Some(now - 60),
            subjects: vec![MemorySubject::profile(
                profile_id.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("memory-secret-due-review".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Hypothesis,
            content: "Secret launch credential should be rotated.".into(),
            source: MemorySource::Reflection,
            importance: 0.9,
            confidence: 0.7,
            sensitivity: 0.95,
            privacy_category: PrivacyCategory::Secret,
            next_review_at: Some(now - 60),
            subjects: vec![MemorySubject::profile(
                profile_id.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: now - 30,
            kind: ThoughtKind::Reflection,
            content: "Alice seemed worried about deployment risk.".into(),
            importance: 0.8,
            confidence: 0.75,
            action_id: Some("action-current-thought".into()),
            memories_accessed: vec![MemoryId("memory-current-profile".into())],
            subjects: vec![
                MemorySubject::profile(profile_id.clone(), Some("about".into()), 1.0),
                MemorySubject::person(person_id.clone(), Some("about".into()), 1.0),
            ],
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: now - 20,
            kind: ThoughtKind::Reflection,
            content: "Bob seemed worried about hiring risk.".into(),
            importance: 0.95,
            confidence: 0.95,
            action_id: Some("action-other-thought".into()),
            memories_accessed: vec![],
            subjects: vec![MemorySubject::profile(
                ProfileId("profile-bob".into()),
                Some("about".into()),
                1.0,
            )],
        })
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-current-followup".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Ask Alice whether the deployment finished cleanly".into(),
            person: Some(person_id.clone()),
            profile: Some(profile_id.clone()),
            conversation: Some(conversation.clone()),
            fire_at: Some(now + 3600),
            condition: None,
            recurrence: None,
            priority: 75,
            dedupe_key: Some("followup:alice:deployment".into()),
            source_action: None,
            source_memory: Some(MemoryId("memory-deploy-followup".into())),
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-other-person".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Ask Bob about unrelated hiring updates".into(),
            person: Some(PersonId("person-bob".into())),
            profile: None,
            conversation: None,
            fire_at: Some(now + 1800),
            condition: None,
            recurrence: None,
            priority: 100,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();

    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let mut actor = ActorState::new(CoreTraits::default());
    actor.set_relationship_config(&person_id, Some(RelationshipStanding::Default));
    if let Some(rel) = actor.bonds.get_mut(&person_id) {
        rel.response_cadence = Some("reply within one business day".into());
        rel.channel_preference = Some("Discord for deployment coordination".into());
    }
    actor.apply_delta(
        &Delta {
            relationship_signal_updates: vec![RelationshipSignalUpdate {
                person: person_id.clone(),
                closeness_delta: 0.4,
                reliability_delta: 0.7,
                reciprocity_delta: 0.5,
                conflict_delta: 0.1,
                reason: "prompt test signals".into(),
            }],
            ..Delta::default()
        },
        &GrowthConfig::default(),
    );
    let shared = Arc::new(SharedState {
        actor: RwLock::new(actor),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state = StateHandle::new(shared, delta_tx);
    let ctx = SessionContext {
        action_id: ActionId("prompt-test".into()),
        kind: SessionKind::Action(ActionKind::Respond),
        messages: vec![inbound],
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
    ctx.typing.write().unwrap().insert(
        (conversation.clone(), "discord".into(), "author-a".into()),
        chrono::Utc::now().timestamp(),
    );

    let prompt = build_system_prompt(
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

    assert!(prompt.contains("## Active directives"));
    assert!(prompt.contains("Use the group norm: keep deployment updates brief."));
    assert!(prompt.contains("## Current conversation"));
    assert!(prompt.contains("Alice asked for concise deployment status in this channel."));
    assert!(prompt.contains("## Current group"));
    assert!(prompt.contains("- id: discord:guild-1"));
    assert!(prompt.contains("- name: Deploy Guild"));
    assert!(prompt.contains("- gateway: discord"));
    assert!(prompt.contains("- external id: guild-1"));
    assert!(prompt.contains("- context: work"));
    assert!(prompt.contains("- observed member count: 3"));
    assert!(prompt.contains("person-eve (Eve)"));
    assert!(prompt.contains("Do not treat group membership or display names as proof"));
    assert!(prompt.contains("## Timing and delivery"));
    assert!(prompt.contains("Gateway: discord (unregistered, not connected)"));
    assert!(prompt.contains("Currently typing:"));
    assert!(prompt.contains("discord:author-a (current sender), active for"));
    assert!(prompt.contains("## Safety boundaries"));
    assert!(prompt.contains("Relationship standing: default"));
    assert!(prompt.contains("Sensitive memory access: conservative recall only"));
    assert!(
        prompt.contains(
            "Third-party outreach: third-party outreach requires a verified active target"
        )
    );
    assert!(prompt.contains("## Recent conversation"));
    assert!(prompt.contains("user [author-a] discord-msg-0: previous deploy thread"));
    assert!(!prompt.contains("discord-msg-1: deploy status?"));
    assert!(prompt.contains(
        "Relationship signals: closeness 40%, reliability 70%, reciprocity 50%, conflict 10%"
    ));
    assert!(prompt.contains("Response cadence preference: reply within one business day"));
    assert!(prompt.contains("Channel preference: Discord for deployment coordination"));
    assert!(prompt.contains("Alice coordinates deployment releases."));
    assert!(prompt.contains("Prefers practical release notes with direct status."));
    assert!(prompt.contains("## Relevant memories"));
    assert!(prompt.contains(
        "[memory-current-profile, current_profile, fact, stated, importance 90%, confidence 80%]"
    ));
    assert!(prompt.contains("Alice prefers brief deployment status updates."));
    assert!(prompt.contains("## Relationship memory pack"));
    assert!(
        prompt.contains(
            "[memory-current-boundary, current_profile, boundary, stated, importance 85%, confidence 90%]"
        )
    );
    assert!(prompt.contains("Do not mention surprise party details in shared channels."));
    assert!(!prompt.contains("Other profile wants verbose deployment status updates."));
    assert!(!prompt.contains("Alice has a secret deployment credential."));
    assert!(prompt.contains("## Social context"));
    assert!(
        prompt
            .contains("person-alice (Alice) -> person-bob (Bob): coworker direction=bidirectional")
    );
    assert!(
        prompt.contains(
            "(confirmed, confidence 80%, source stated, asserted by person-alice (Alice), evidence message social-msg-1)"
        )
    );
    assert!(
        prompt.contains(
            "person-carol (Carol) -> person-alice (Alice): friend direction=bidirectional"
        )
    );
    assert!(prompt.contains(
        "(hypothesis, confidence 45%, source inferred, evidence reason: seen together in channel)"
    ));
    assert!(!prompt.contains("person-alice (Alice) -> person-dave (Dave): friend"));
    assert!(!prompt.contains("person-bob (Bob) -> person-dave (Dave): friend"));
    assert!(prompt.contains("## Open loops"));
    assert!(prompt.contains("intent-current-followup"));
    assert!(prompt.contains("Ask Alice whether the deployment finished cleanly"));
    assert!(prompt.contains("priority 75"));
    assert!(prompt.contains("source memory memory-deploy-followup"));
    assert!(!prompt.contains("## Memories due for review"));
    assert!(!prompt.contains("Old launch checklist may be stale."));
    assert!(!prompt.contains("Ask Bob about unrelated hiring updates"));
    assert!(prompt.contains("## Recent thoughts"));
    assert!(prompt.contains("Alice seemed worried about deployment risk."));
    assert!(!prompt.contains("Bob seemed worried about hiring risk."));
    assert!(!prompt.contains("## Conversation summary backlog"));
    assert!(prompt.contains("A new message came in."));
    assert!(!prompt.contains("Post-turn review."));

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
    assert!(mind_prompt.contains("## Social context"));
    assert!(
        mind_prompt
            .contains("person-alice (Alice) -> person-bob (Bob): coworker direction=bidirectional")
    );
    assert!(mind_prompt.contains("asserted by person-alice (Alice)"));
    assert!(mind_prompt.contains("evidence message social-msg-1"));
    assert!(!mind_prompt.contains("person-bob (Bob) -> person-dave (Dave): friend"));
    assert!(mind_prompt.contains("## Relationship memory pack"));
    assert!(mind_prompt.contains("Do not mention surprise party details in shared channels."));
    assert!(mind_prompt.contains("## Relevant memories"));
    assert!(mind_prompt.contains(
        "[memory-current-profile, current_profile, fact, stated, importance 90%, confidence 80%]"
    ));
    assert!(mind_prompt.contains("Alice prefers brief deployment status updates."));
    assert!(!mind_prompt.contains("Other profile wants verbose deployment status updates."));
    assert!(!mind_prompt.contains("Alice has a secret deployment credential."));
    assert!(mind_prompt.contains("## Current conversation"));
    assert!(mind_prompt.contains("Alice asked for concise deployment status in this channel."));
    assert!(mind_prompt.contains("## Current group"));
    assert!(mind_prompt.contains("- id: discord:guild-1"));
    assert!(mind_prompt.contains("- name: Deploy Guild"));
    assert!(mind_prompt.contains("- context: work"));
    assert!(mind_prompt.contains("- observed member count: 3"));
    assert!(mind_prompt.contains("person-eve (Eve)"));
    assert!(mind_prompt.contains("Do not treat group membership or display names as proof"));
    assert!(mind_prompt.contains("## Current profile"));
    assert!(mind_prompt.contains("Alice's Discord profile tracks deployment coordination."));
    assert!(mind_prompt.contains("On Discord, Alice prefers terse release status."));
    assert!(mind_prompt.contains("linked person id: person-alice"));
    assert!(mind_prompt.contains("Communication style: Prefers practical release notes"));
    assert!(mind_prompt.contains("## Timing and delivery"));
    assert!(mind_prompt.contains("Gateway: discord (unregistered, not connected)"));
    assert!(mind_prompt.contains("Currently typing:"));
    assert!(mind_prompt.contains("discord:author-a (current sender), active for"));
    assert!(mind_prompt.contains("## Safety boundaries"));
    assert!(mind_prompt.contains(
        "Relationship signals: closeness 40%, reliability 70%, reciprocity 50%, conflict 10%"
    ));
    assert!(mind_prompt.contains("Response cadence preference: reply within one business day"));
    assert!(mind_prompt.contains("Channel preference: Discord for deployment coordination"));
    assert!(mind_prompt.contains("## Recent conversation"));
    assert!(mind_prompt.contains("user [author-a] discord-msg-0: previous deploy thread"));
    assert!(!mind_prompt.contains("discord-msg-1: deploy status?"));
    assert!(mind_prompt.contains("## Open loops"));
    assert!(mind_prompt.contains("intent-current-followup"));
    assert!(mind_prompt.contains("source memory memory-deploy-followup"));
    assert!(!mind_prompt.contains("Ask Bob about unrelated hiring updates"));
    assert!(mind_prompt.contains("## Recent thoughts"));
    assert!(mind_prompt.contains("Alice seemed worried about deployment risk."));
    assert!(!mind_prompt.contains("Bob seemed worried about hiring risk."));

    let consolidate_kind = SessionKind::Action(ActionKind::Consolidate);
    let consolidate_prompt = build_system_prompt(
        &state,
        &store_dyn,
        &consolidate_kind,
        &ctx.messages,
        Some(&conversation),
        &ctx,
        &RelationshipStanding::Default,
    )
    .await
    .unwrap();
    assert!(consolidate_prompt.contains("## Memories due for review"));
    assert!(consolidate_prompt.contains("memory-due-review"));
    assert!(consolidate_prompt.contains("Old launch checklist may be stale."));
    assert!(consolidate_prompt.contains("overdue by 1 minute"));
    assert!(consolidate_prompt.contains("memory-secret-due-review"));
    assert!(consolidate_prompt.contains("sensitive memory content redacted"));
    assert!(!consolidate_prompt.contains("Secret launch credential should be rotated."));
    assert!(consolidate_prompt.contains("## Conversation summary backlog"));
    assert!(consolidate_prompt.contains("discord:channel-1"));
    assert!(consolidate_prompt.contains("1 uncovered message of 2 total"));
    assert!(
        consolidate_prompt.contains("Alice asked for concise deployment status in this channel.")
    );
}
