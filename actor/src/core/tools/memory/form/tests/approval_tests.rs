use super::*;

#[tokio::test]
async fn form_memory_can_store_chosen_human_approved_actor_self_memory() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let router = InferenceRouterBuilder::new()
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
            model: "noop".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Chat],
            reasoning: Reasoning::Basic,
        })
        .build()
        .unwrap();
    let ctx = SessionContext {
        action_id: ActionId("actor-self-memory-test".into()),
        kind: SessionKind::Action(ActionKind::Respond),
        messages: vec![],
        conversation: None,
        relationship_standing: RelationshipStanding::ChosenHuman,
        style_directive: None,
        cancelled_note: None,
        concurrent_summaries: vec![],
        state: StateHandle::new(shared, delta_tx),
        store: store_dyn,
        media_store: None,
        router: Arc::new(router),
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
            "content": "My name is Pamagotchi.",
            "kind": "semantic",
            "memory_type": "identity_claim",
            "sensitivity_category": "identity",
            "subject_actor": true
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
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Actor);
    assert_eq!(memory.subjects[0].subject_id, "self");
    assert_eq!(memory.memory_type, MemoryType::IdentityClaim);

    let recalled = store
        .recall(&RecallQuery::by_text("my name", 10).with_actor_subject())
        .await
        .unwrap();
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, state.memories_formed[0]);
}
