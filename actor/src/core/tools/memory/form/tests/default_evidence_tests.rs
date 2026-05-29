use super::*;

#[tokio::test]
async fn form_memory_defaults_evidence_to_current_action_messages() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let conversation = ConversationId("relay:local".into());
    let profile = ProfileId("profile-sam".into());
    let identity = IdentityId("identity-local".into());
    let other_profile = ProfileId("profile-alice".into());
    let other_identity = IdentityId("identity-alice".into());
    store
        .store_memory(&Memory {
            id: MemoryId("old-memory".into()),
            kind: MemoryKind::Semantic,
            content: "Sam prefers long deployment updates.".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    let messages = vec![
        InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: conversation.clone(),
            group: None,
            identity: Some(identity.clone()),
            profile: Some(profile.clone()),
            person: None,
            content: "I prefer short deploy updates.".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        },
        InboundMessage {
            message_id: "msg-2".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: conversation.clone(),
            group: None,
            identity: Some(other_identity.clone()),
            profile: Some(other_profile.clone()),
            person: None,
            content: "Actually, concise is best.".into(),
            attachments: vec![],
            timestamp: 1001,
            metadata: serde_json::Value::Null,
        },
    ];
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
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(EmbeddingBridge)),
            model: "embed-test".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Embedding],
            reasoning: Reasoning::Basic,
        })
        .build()
        .unwrap();
    let ctx = SessionContext {
        action_id: ActionId("form-memory-test".into()),
        kind: SessionKind::Action(ActionKind::Review),
        messages,
        conversation: Some(conversation.clone()),
        authority: Authority::Default,
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
            "content": "Sam prefers concise deployment updates.",
            "kind": "semantic",
            "memory_type": "correction",
            "truth_status": "confirmed",
            "tags": ["preference", "deployment"],
            "confidence": 0.87,
            "emotional_valence": 0.2,
            "supersedes": "old-memory",
            "contradiction_group": "deploy-update-length",
            "evidence_quote": "I prefer short deploy updates.",
            "source_spans": [{
                "message_id": "msg-1",
                "start_char": 0,
                "end_char": 30,
                "quote": "I prefer short deploy updates."
            }],
            "evidence": {"reason": "user corrected preference"},
            "expires_at": 3234,
            "stability": "stable",
            "last_confirmed_at": 1234,
            "next_review_at": 2234,
            "dedupe_key": "correction:profile-sam:concise-deployment-updates"
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
    assert_eq!(memory.evidence_message_ids, vec!["msg-1", "msg-2"]);
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, "profile-sam");
    assert_eq!(memory.memory_type, MemoryType::Correction);
    assert_eq!(memory.truth_status, TruthStatus::Confirmed);
    assert_eq!(memory.tags, vec!["preference", "deployment"]);
    assert_eq!(memory.confidence, 0.87);
    assert_eq!(memory.emotional_valence, 0.2);
    assert_eq!(
        memory.supersedes.as_ref().map(|id| id.0.as_str()),
        Some("old-memory")
    );
    assert_eq!(
        memory.contradiction_group.as_deref(),
        Some("deploy-update-length")
    );
    assert_eq!(memory.evidence["reason"], "user corrected preference");
    assert_eq!(memory.evidence["source_spans"][0]["message_id"], "msg-1");
    assert_eq!(
        memory.evidence["source_spans"][0]["quote"],
        "I prefer short deploy updates."
    );
    assert_eq!(
        memory.evidence_quote.as_deref(),
        Some("I prefer short deploy updates.")
    );
    assert_eq!(memory.expires_at, Some(3234));
    assert_eq!(memory.stability, MemoryStability::Stable);
    assert_eq!(memory.last_confirmed_at, Some(1234));
    assert_eq!(memory.next_review_at, Some(2234));
    assert_eq!(
        memory.dedupe_key.as_deref(),
        Some("correction:profile-sam:concise-deployment-updates")
    );
    assert_eq!(memory.embedding_model.as_deref(), Some("embed-test"));
    assert_eq!(memory.embedding.as_deref(), Some(&[0.1, 0.2, 0.3, 0.4][..]));
    let old_memory = store
        .get_memory(&MemoryId("old-memory".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        old_memory.superseded_by,
        Some(state.memories_formed[0].clone())
    );
    assert_eq!(old_memory.truth_status, TruthStatus::Outdated);
    match memory.source {
        MemorySource::Conversation {
            conversation_id,
            identity_id,
            profile_id,
            message_id,
            ..
        } => {
            assert_eq!(conversation_id, conversation);
            assert_eq!(identity_id, Some(identity));
            assert_eq!(profile_id, Some(profile));
            assert_eq!(message_id.as_deref(), Some("msg-1"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }

    let result = form(
        &json!({
            "content": "Alice prefers concise release notes.",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "evidence_message_ids": ["msg-2"],
            "dedupe_key": "preference:profile-alice:concise-release-notes"
        }),
        &ctx,
        &mut state,
    )
    .await;

    assert!(result.starts_with("Memory saved: "));
    let alice_memory_id = state.memories_formed.last().unwrap().clone();
    let memory = store.get_memory(&alice_memory_id).await.unwrap().unwrap();
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, "profile-alice");
    match memory.source {
        MemorySource::Conversation {
            identity_id,
            profile_id,
            message_id,
            ..
        } => {
            assert_eq!(identity_id, Some(other_identity));
            assert_eq!(profile_id, Some(other_profile));
            assert_eq!(message_id.as_deref(), Some("msg-2"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }

    let result = form(
        &json!({
            "content": "Alice prefers concise release notes with chosen_people.",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "confirmed",
            "evidence_message_ids": ["msg-2"],
            "dedupe_key": "preference:profile-alice:concise-release-notes"
        }),
        &ctx,
        &mut state,
    )
    .await;

    assert!(result.starts_with("Memory saved: "));
    assert_eq!(state.memories_formed.last(), Some(&alice_memory_id));
    let memory = store.get_memory(&alice_memory_id).await.unwrap().unwrap();
    assert!(
        memory
            .content
            .contains("Alice prefers concise release notes with chosen_people.")
    );
    assert_eq!(memory.truth_status, TruthStatus::Confirmed);

    let metrics = ctx.metrics.snapshot();
    assert_eq!(metrics.memory_created, 2);
    assert_eq!(metrics.memory_updated, 2);
    assert_eq!(metrics.memory_superseded, 1);

    let recall_result = super::super::super::recall::recall(
        &json!({
            "query": "concise deployment updates",
            "limit": 5,
            "include_sensitive": true,
            "include_superseded": true
        }),
        &ctx,
        &mut state,
    )
    .await;
    let recalled: serde_json::Value = serde_json::from_str(&recall_result).unwrap();
    let recalled_memory = recalled["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_str() == Some(state.memories_formed[0].0.as_str()))
        .expect("formed memory is recalled");
    assert_eq!(recalled_memory["memory_type"], "correction");
    assert_eq!(recalled_memory["truth_status"], "confirmed");
    assert!((recalled_memory["confidence"].as_f64().unwrap() - 0.87).abs() < 0.000001);
    assert!((recalled_memory["emotional_valence"].as_f64().unwrap() - 0.2).abs() < 0.000001);
    assert_eq!(recalled_memory["tags"], json!(["preference", "deployment"]));
    assert_eq!(
        recalled_memory["evidence_message_ids"],
        json!(["msg-1", "msg-2"])
    );
    assert_eq!(
        recalled_memory["evidence_quote"],
        "I prefer short deploy updates."
    );
    assert_eq!(
        recalled_memory["evidence"]["reason"],
        "user corrected preference"
    );
    assert_eq!(recalled_memory["expires_at"], 3234);
    assert_eq!(recalled_memory["stability"], "stable");
    assert_eq!(
        recalled_memory["dedupe_key"],
        "correction:profile-sam:concise-deployment-updates"
    );
    assert_eq!(recalled_memory["last_confirmed_at"], 1234);
    assert_eq!(recalled_memory["next_review_at"], 2234);

    let result = form(
        &json!({
            "content": "Alice's deployment credential should be rotated.",
            "kind": "semantic",
            "memory_type": "procedure",
            "sensitivity": 0.95,
            "sensitivity_category": "credentials",
            "evidence_message_ids": ["msg-2"]
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
    assert_eq!(memory.privacy_category, PrivacyCategory::Secret);
    assert_eq!(memory.visibility_scope, VisibilityScope::ChosenHumanOnly);
    assert!(memory.next_review_at.is_some());
}
