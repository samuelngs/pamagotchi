use super::*;

#[tokio::test]
async fn review_prompt_includes_source_action_transcript() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let conversation = ConversationId("relay:local".into());
    let source_action = "source-action-1";

    store
        .start_action_run(&ActionRunRecord {
            action_id: source_action.into(),
            kind: "respond".into(),
            task: "Respond to message".into(),
            conversation: Some(conversation.clone()),
            started_at: 1000,
            ended_at: None,
            status: "running".into(),
            responded: false,
            attempts: 0,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: source_action.into(),
            role: "user".into(),
            conversation: Some(conversation.clone()),
            source_gateway_id: Some("relay".into()),
            source_message_id: Some("msg-1".into()),
            sender_external_id: Some("local".into()),
            reply_external_id: Some("local".into()),
            content: Some("hello".into()),
            created_at: 1001,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: source_action.into(),
            role: "assistant".into(),
            conversation: Some(conversation.clone()),
            source_gateway_id: None,
            source_message_id: None,
            sender_external_id: None,
            reply_external_id: Some("local".into()),
            content: Some("hi there".into()),
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .append_action_turn(&ActionTurnRecord {
            action_id: source_action.into(),
            turn: 0,
            attempt: 1,
            prompt_hash: "abc123".into(),
            model: Some("model-a".into()),
            finish: Some("tool_calls".into()),
            input_tokens: Some(20),
            output_tokens: Some(5),
            text_len: 0,
            reasoning_len: 0,
            tool_call_count: 1,
            created_at: 1002,
        })
        .await
        .unwrap();
    store
        .append_tool_call(&ToolCallRecord {
            action_id: source_action.into(),
            turn: 0,
            call_id: "call-1".into(),
            name: "send_message".into(),
            args: serde_json::json!({"content": "hi there"}),
            result: serde_json::json!({"result": "Message sent."}),
            success: true,
            started_at: 1003,
            ended_at: 1004,
        })
        .await
        .unwrap();
    store
        .log_thought(&Thought {
            timestamp: 1003,
            kind: ThoughtKind::Observation,
            content: "Sam may prefer shorter greetings.".into(),
            importance: 0.8,
            confidence: 0.7,
            action_id: Some(source_action.into()),
            memories_accessed: vec![MemoryId("memory-greeting-style".into())],
            subjects: vec![],
        })
        .await
        .unwrap();
    store
        .append_outbound_delivery(&OutboundDeliveryRecord {
            action_id: source_action.into(),
            conversation: Some(conversation.clone()),
            message: None,
            channel: None,
            gateway_id: "relay".into(),
            external_id: "local".into(),
            status: "delivered".into(),
            error: None,
            attempted_at: 1004,
        })
        .await
        .unwrap();
    store
        .finish_action_run(
            source_action,
            1005,
            "completed",
            true,
            1,
            vec![MemoryId("memory-created-review".into())],
            vec![MemoryId("memory-greeting-style".into())],
        )
        .await
        .unwrap();

    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(CoreTraits::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state = StateHandle::new(shared, delta_tx);
    let ctx = SessionContext {
        action_id: ActionId("review-action".into()),
        kind: SessionKind::Action(ActionKind::Review),
        messages: vec![],
        conversation: Some(conversation.clone()),
        relationship_standing: RelationshipStanding::Default,
        style_directive: None,
        cancelled_note: Some(format!("Post-turn review for action {source_action}")),
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
        &RelationshipStanding::Default,
    )
    .await
    .unwrap();

    assert!(prompt.contains("## Source action transcript"));
    assert!(prompt.contains("source-action-1"));
    assert!(prompt.contains("- task: Respond to message"));
    assert!(prompt.contains("- status: completed, responded: true, attempts: 1"));
    assert!(prompt.contains("user [local] msg-1: hello"));
    assert!(prompt.contains("assistant [local]: hi there"));
    assert!(prompt.contains("attempt 1, turn 0, model model-a"));
    assert!(prompt.contains("send_message success=true"));
    assert!(prompt.contains("Message sent."));
    assert!(prompt.contains("### Source action thoughts"));
    assert!(prompt.contains("Sam may prefer shorter greetings."));
    assert!(prompt.contains("memories: memory-greeting-style"));
    assert!(prompt.contains("### Outcome memory trace"));
    assert!(prompt.contains("formed memories: memory-created-review"));
    assert!(prompt.contains("recalled memories: memory-greeting-style"));
    assert!(prompt.contains("delivery relay:local: delivered"));
    assert!(prompt.contains("Tool arguments must be strict JSON"));
    assert!(prompt.contains(r#"{"conversation_summary":{"summary":"...""#));
}
