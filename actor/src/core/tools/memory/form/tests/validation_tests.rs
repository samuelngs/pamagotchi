use super::*;

#[test]
fn memory_numeric_fields_are_clamped() {
    assert_eq!(clamp_unit(-0.2), 0.0);
    assert_eq!(clamp_unit(1.2), 1.0);
    assert_eq!(clamp_valence(-1.2), -1.0);
    assert_eq!(clamp_valence(1.2), 1.0);
}
#[tokio::test]
async fn form_memory_rejects_unavailable_explicit_evidence_message_ids() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let conversation = ConversationId("relay:local".into());
    let profile = ProfileId("profile-sam".into());
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let ctx = SessionContext {
        action_id: ActionId("form-memory-missing-evidence-test".into()),
        kind: SessionKind::Action(ActionKind::Review),
        messages: vec![InboundMessage {
            message_id: "msg-present".into(),
            gateway_id: "relay".into(),
            sender: Some(protocol::ObservedSender::primary(
                "relay", "local", None, "test",
            )),
            channel: protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct),
            conversation: conversation.clone(),
            identity: None,
            profile: Some(profile),
            person: None,
            content: "I prefer concise launch briefs.".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        }],
        conversation: Some(conversation),
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
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    let result = form(
        &json!({
            "content": "Sam prefers concise launch briefs.",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "evidence_message_ids": ["msg-missing"]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let value: Value = serde_json::from_str(&result).unwrap();

    assert!(value["error"].as_str().unwrap().contains("not available"));
    assert!(state.memories_formed.is_empty());
    assert!(
        store
            .recall(&RecallQuery::by_text("concise launch briefs", 5))
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn form_memory_accepts_read_message_evidence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let conversation = ConversationId("relay:local".into());
    let profile = ProfileId("profile-sam".into());
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let ctx = SessionContext {
        action_id: ActionId("form-memory-read-evidence-test".into()),
        kind: SessionKind::Action(ActionKind::Consolidate),
        messages: vec![],
        conversation: Some(conversation.clone()),
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
    let read_message = InboundMessage {
        message_id: "msg-read".into(),
        gateway_id: "relay".into(),
        sender: Some(protocol::ObservedSender::primary(
            "relay", "local", None, "test",
        )),
        channel: protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct),
        conversation: conversation.clone(),
        identity: None,
        profile: Some(profile.clone()),
        person: None,
        content: "I prefer concise rollout notes.".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: serde_json::Value::Null,
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
        presented_injected_messages: vec![],
        presented_read_messages: vec![read_message],
        pending_injected_messages: vec![],
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    let result = form(
        &json!({
            "content": "Sam prefers concise rollout notes.",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "evidence_message_ids": ["msg-read"]
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
    assert_eq!(memory.evidence_message_ids, vec!["msg-read"]);
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, profile.0);
    match memory.source {
        MemorySource::Conversation { message_id, .. } => {
            assert_eq!(message_id.as_deref(), Some("msg-read"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }
}
#[tokio::test]
async fn form_memory_defaults_uncertain_and_emotional_memories_to_transient() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let conversation = ConversationId("relay:local".into());
    let profile = ProfileId("profile-sam".into());
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let ctx = SessionContext {
        action_id: ActionId("form-memory-transient-defaults-test".into()),
        kind: SessionKind::Action(ActionKind::Review),
        messages: vec![InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "relay".into(),
            sender: Some(protocol::ObservedSender::primary(
                "relay", "local", None, "test",
            )),
            channel: protocol::ChannelKey::new("relay", "local", protocol::ChannelKind::Direct),
            conversation: conversation.clone(),
            identity: None,
            profile: Some(profile),
            person: None,
            content: "I might be annoyed about launch today.".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        }],
        conversation: Some(conversation),
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
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: Default::default(),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    let result = form(
        &json!({
            "content": "Sam might be annoyed about launch today.",
            "kind": "semantic",
            "memory_type": "hypothesis",
            "stability": "stable",
            "evidence_message_ids": ["msg-1"],
            "dedupe_key": "hypothesis:profile-sam:annoyed-launch"
        }),
        &ctx,
        &mut state,
    )
    .await;
    assert!(result.starts_with("Memory saved: "));
    let hypothesis = store
        .get_memory(state.memories_formed.last().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hypothesis.memory_type, MemoryType::Hypothesis);
    assert_eq!(hypothesis.truth_status, TruthStatus::Inferred);
    assert_eq!(hypothesis.stability, MemoryStability::Transient);

    let result = form(
        &json!({
            "content": "Sam feels annoyed about launch today.",
            "kind": "episodic",
            "memory_type": "emotional_state",
            "truth_status": "stated",
            "stability": "stable",
            "evidence_message_ids": ["msg-1"],
            "dedupe_key": "emotion:profile-sam:annoyed-launch"
        }),
        &ctx,
        &mut state,
    )
    .await;
    assert!(result.starts_with("Memory saved: "));
    let emotion = store
        .get_memory(state.memories_formed.last().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(emotion.memory_type, MemoryType::EmotionalState);
    assert_eq!(emotion.truth_status, TruthStatus::Stated);
    assert_eq!(emotion.stability, MemoryStability::Transient);
}
