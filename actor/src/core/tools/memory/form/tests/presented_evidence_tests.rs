use super::*;

#[tokio::test]
async fn form_memory_uses_presented_injected_message_evidence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let source_conversation = ConversationId("relay:source".into());
    let injected_conversation = ConversationId("relay:injected".into());
    let source_profile = ProfileId("profile-source".into());
    let source_identity = IdentityId("identity-source".into());
    let injected_profile = ProfileId("profile-injected".into());
    let injected_identity = IdentityId("identity-injected".into());
    let source_message = InboundMessage {
        message_id: "msg-source".into(),
        gateway_id: "relay".into(),
        sender: Some(protocol::ObservedSender::primary(
            "relay", "source", None, "test",
        )),
        channel: protocol::ChannelKey::new("relay", "source", protocol::ChannelKind::Direct),
        conversation: source_conversation.clone(),
        identity: Some(source_identity.clone()),
        profile: Some(source_profile.clone()),
        person: None,
        content: "I prefer terse status notes.".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: serde_json::Value::Null,
    };
    let injected_message = InboundMessage {
        message_id: "msg-injected".into(),
        gateway_id: "relay".into(),
        sender: Some(protocol::ObservedSender::primary(
            "relay", "injected", None, "test",
        )),
        channel: protocol::ChannelKey::new("relay", "injected", protocol::ChannelKind::Direct),
        conversation: injected_conversation.clone(),
        identity: Some(injected_identity.clone()),
        profile: Some(injected_profile.clone()),
        person: None,
        content: "For release notes, include the chosen human and rollback path.".into(),
        attachments: vec![],
        timestamp: 1001,
        metadata: serde_json::Value::Null,
    };
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let ctx = SessionContext {
        action_id: ActionId("form-memory-injected-evidence-test".into()),
        kind: SessionKind::Action(ActionKind::Review),
        messages: vec![source_message],
        conversation: None,
        relationship_standing: RelationshipStanding::Default,
        style_directive: None,
        cancelled_note: None,
        concurrent_summaries: vec![],
        state: StateHandle::new(shared, delta_tx),
        store: store_dyn,
        media_store: None,
        router: Arc::new(router_with_failing_embedding_endpoint()),
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
    let mut state = SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: Delta::default(),
        thoughts: vec![],
        memories_formed: vec![],
        recalled_memory_ids: vec![],
        injected_messages: vec![],
        presented_injected_messages: vec![injected_message],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 1,
    };

    let result = form(
        &json!({
            "content": "The source profile prefers terse status notes.",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "dedupe_key": "preference:profile-source:terse-status-notes"
        }),
        &ctx,
        &mut state,
    )
    .await;

    assert!(result.starts_with("Memory saved: "));
    let memory = store
        .get_memory(&state.memories_formed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        memory.evidence_message_ids,
        vec!["msg-source", "msg-injected"]
    );

    let result = form(
        &json!({
            "content": "The injected profile wants release notes to include chosen_people and rollback paths.",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "evidence_message_ids": ["msg-injected"],
            "dedupe_key": "preference:profile-injected:release-note-chosen_human-rollback"
        }),
        &ctx,
        &mut state,
    )
    .await;

    assert!(result.starts_with("Memory saved: "));
    let memory = store
        .get_memory(state.memories_formed.last().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, injected_profile.0);
    match memory.source {
        MemorySource::Conversation {
            conversation_id,
            identity_id,
            profile_id,
            message_id,
            ..
        } => {
            assert_eq!(conversation_id, injected_conversation);
            assert_eq!(identity_id, Some(injected_identity));
            assert_eq!(profile_id, Some(injected_profile));
            assert_eq!(message_id.as_deref(), Some("msg-injected"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }
}
