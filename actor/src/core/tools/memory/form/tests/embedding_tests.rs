use super::*;

#[tokio::test]
async fn form_memory_persists_without_embedding_when_embedding_endpoint_fails() {
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
        action_id: ActionId("form-memory-embedding-failure-test".into()),
        kind: SessionKind::Action(ActionKind::Review),
        messages: vec![InboundMessage {
            message_id: "msg-embed-fail".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: conversation.clone(),
            group: None,
            identity: None,
            profile: Some(profile.clone()),
            person: None,
            content: "I prefer concise launch briefs.".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        }],
        conversation: Some(conversation),
        authority: Authority::Default,
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
            "evidence_message_ids": ["msg-embed-fail"]
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
    assert!(memory.embedding.is_none());
    assert_eq!(memory.memory_type, MemoryType::Preference);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, profile.0);
}
