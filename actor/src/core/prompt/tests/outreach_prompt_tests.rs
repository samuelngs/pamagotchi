use super::*;

#[tokio::test]
async fn outreach_prompt_uses_conversation_target_context_without_current_messages() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let identity_id = IdentityId("identity-outreach".into());
    let profile_id = ProfileId("profile-outreach".into());
    let person_id = PersonId("person-outreach".into());
    let conversation = ConversationId("relay:outreach".into());
    let group = GroupId("relay:team-chat".into());
    let now = chrono::Utc::now().timestamp();

    store
        .add_identity(&Identity {
            id: identity_id.clone(),
            gateway_id: "relay".into(),
            external_id: "local".into(),
            display_name: Some("Sam".into()),
            metadata: None,
            created_at: now,
            last_seen_at: now,
        })
        .await
        .unwrap();
    store
        .add_profile(&Profile {
            id: profile_id.clone(),
            display_name: Some("Sam relay".into()),
            summary: Some("Profile summary for outreach.".into()),
            comm_style: Some("Profile prefers short scheduling messages.".into()),
            first_seen: now,
            last_seen: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .link_identity_to_profile(&identity_id, &profile_id, 1.0, None)
        .await
        .unwrap();
    store
        .add_person(&Person {
            id: person_id.clone(),
            name: Some("Sam".into()),
            summary: Some("Person summary for outreach.".into()),
            comm_style: Some("Person-level style for verified outreach.".into()),
            first_seen: now,
            last_seen: now,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile_id,
            &person_id,
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();
    store
        .add_group(&Group {
            id: group.clone(),
            name: "Relay Team Chat".into(),
            gateway_id: "relay".into(),
            external_id: "team-chat".into(),
            context: GroupContext::Work,
            members: vec![person_id.clone()],
        })
        .await
        .unwrap();
    store
        .append_message(
            &conversation,
            Some("relay"),
            Some(&group),
            &StoredMessage {
                timestamp: now,
                role: MessageRole::User,
                content: "please check in tomorrow".into(),
                identity: Some(identity_id.clone()),
                profile: Some(profile_id.clone()),
                person: Some(person_id.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-outreach-source".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .update_conversation_summary(
            &conversation,
            "Sam asked for a proactive follow-up.",
            &["msg-outreach-source".into()],
        )
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("memory-deployment-checklist".into()),
            content: "Sam wants deployment checklist follow-ups to mention release readiness."
                .into(),
            importance: 0.8,
            confidence: 0.9,
            subjects: vec![
                MemorySubject::profile(profile_id.clone(), Some("about".into()), 1.0),
                MemorySubject::person(person_id.clone(), Some("about".into()), 1.0),
            ],
            ..Memory::default()
        })
        .await
        .unwrap();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "outreach-prompt-test".into(),
            kind: "outreach".into(),
            task: "Ask Sam whether the deployment checklist is ready".into(),
            conversation: Some(conversation.clone()),
            started_at: now,
            ended_at: None,
            status: "running".into(),
            responded: false,
            attempts: 0,
        })
        .await
        .unwrap();

    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let mut actor = ActorState::new(CoreTraits::default());
    actor.set_relationship_config(&person_id, Some(Authority::Default));
    if let Some(rel) = actor.bonds.get_mut(&person_id) {
        rel.response_cadence = Some("reply within one business day".into());
        rel.channel_preference = Some("relay for proactive check-ins".into());
    }
    let shared = Arc::new(SharedState {
        actor: RwLock::new(actor),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state = StateHandle::new(shared, delta_tx);
    let ctx = SessionContext {
        action_id: ActionId("outreach-prompt-test".into()),
        kind: SessionKind::Action(ActionKind::Outreach),
        messages: vec![],
        conversation: Some(conversation.clone()),
        authority: Authority::Default,
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

    let prompt = build_system_prompt(
        &state,
        &store_dyn,
        &ctx.kind,
        &ctx.messages,
        Some(&conversation),
        &ctx,
        &Authority::Default,
    )
    .await
    .unwrap();

    assert!(prompt.contains("## Current gateway identity"));
    assert!(prompt.contains("identity-outreach"));
    assert!(prompt.contains("Profile summary for outreach."));
    assert!(prompt.contains("Profile prefers short scheduling messages."));
    assert!(prompt.contains("Person summary for outreach."));
    assert!(prompt.contains("## Current group"));
    assert!(prompt.contains("- id: relay:team-chat"));
    assert!(prompt.contains("- name: Relay Team Chat"));
    assert!(prompt.contains("- context: work"));
    assert!(prompt.contains("person-outreach (Sam)"));
    assert!(prompt.contains("Use group membership as local participant context only."));
    assert!(prompt.contains("## Current action"));
    assert!(prompt.contains("- kind: outreach"));
    assert!(prompt.contains("- task: Ask Sam whether the deployment checklist is ready"));
    assert!(prompt.contains("## Recent conversation"));
    assert!(prompt.contains("user [local] msg-outreach-source: please check in tomorrow"));
    assert!(prompt.contains("## Relevant memories"));
    assert!(prompt.contains("memory-deployment-checklist"));
    assert!(prompt.contains("release readiness"));
    assert!(prompt.contains("Response cadence preference: reply within one business day"));
    assert!(prompt.contains("Channel preference: relay for proactive check-ins"));
    assert!(prompt.contains("Sam asked for a proactive follow-up."));
}
