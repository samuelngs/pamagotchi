use super::*;

#[tokio::test]
async fn apply_review_can_forget_noise_memory_with_audit_reason() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    store
        .store_memory(&Memory {
            id: MemoryId("memory-noisy-duplicate".into()),
            kind: MemoryKind::Semantic,
            content: "Noisy duplicate summary fragment.".into(),
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "forget",
                "memory_id": "memory-noisy-duplicate",
                "reason": "review classified this as a noisy duplicate"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    assert!(
        store
            .get_memory(&MemoryId("memory-noisy-duplicate".into()))
            .await
            .unwrap()
            .is_none()
    );
    let mutations = store
        .memory_mutations_for_memory(&MemoryId("memory-noisy-duplicate".into()), 10)
        .await
        .unwrap();
    assert_eq!(mutations[0].operation, "forget");
    assert_eq!(
        mutations[0].reason.as_deref(),
        Some("review classified this as a noisy duplicate")
    );
}
#[tokio::test]
async fn apply_review_can_reinforce_existing_memory_with_evidence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    store
        .store_memory(&Memory {
            id: MemoryId("memory-existing-preference".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Preference,
            truth_status: TruthStatus::Stated,
            content: "Sam prefers concise summaries.".into(),
            confidence: 0.4,
            importance: 0.3,
            evidence_message_ids: vec!["msg-old".into()],
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "reinforce",
                "memory_id": "memory-existing-preference",
                "confidence": 0.7,
                "importance": 0.6,
                "reason": "same preference appeared again",
                "evidence_message_ids": ["msg-1"],
                "evidence_quote": "make future summaries concise"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let memory = store
        .get_memory(&MemoryId("memory-existing-preference".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.confidence, 0.7);
    assert_eq!(memory.importance, 0.6);
    assert_eq!(
        memory.evidence_message_ids,
        vec!["msg-old".to_string(), "msg-1".to_string()]
    );
    assert!(memory.last_confirmed_at.is_some());
    assert_eq!(memory.evidence["operation"], "reinforce");
    assert_eq!(memory.evidence["reason"], "same preference appeared again");

    let mutations = store
        .memory_mutations_for_memory(&MemoryId("memory-existing-preference".into()), 10)
        .await
        .unwrap();
    assert_eq!(mutations[0].operation, "update");
    let fields = mutations[0].data["fields"].as_array().unwrap();
    assert!(fields.contains(&json!("confidence")));
    assert!(fields.contains(&json!("last_confirmed_at")));
    assert!(fields.contains(&json!("evidence")));
}
#[tokio::test]
async fn apply_review_can_update_existing_memory_by_id() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    store
        .store_memory(&Memory {
            id: MemoryId("memory-update-target".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Fact,
            truth_status: TruthStatus::Inferred,
            content: "Sam may prefer verbose summaries.".into(),
            confidence: 0.3,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "update",
                "memory_id": "memory-update-target",
                "content": "Sam prefers concise summaries.",
                "memory_type": "preference",
                "truth_status": "stated",
                "confidence": 0.85,
                "evidence_message_ids": ["msg-1"],
                "reason": "current message corrected the prior inference"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let memory = store
        .get_memory(&MemoryId("memory-update-target".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.content, "Sam prefers concise summaries.");
    assert_eq!(memory.memory_type, MemoryType::Preference);
    assert_eq!(memory.truth_status, TruthStatus::Stated);
    assert_eq!(memory.confidence, 0.85);
    assert_eq!(memory.evidence["operation"], "update");

    let mutations = store
        .memory_mutations_for_memory(&MemoryId("memory-update-target".into()), 10)
        .await
        .unwrap();
    let fields = mutations[0].data["fields"].as_array().unwrap();
    assert!(fields.contains(&json!("content")));
    assert!(fields.contains(&json!("memory_type")));
    assert!(fields.contains(&json!("truth_status")));
}
#[tokio::test]
async fn apply_review_can_mark_existing_memory_contradicted() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    store
        .store_memory(&Memory {
            id: MemoryId("memory-contradicted-target".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Fact,
            truth_status: TruthStatus::Stated,
            content: "Sam lives in Toronto.".into(),
            confidence: 0.8,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "contradict",
                "memory_id": "memory-contradicted-target",
                "reason": "Sam corrected the location",
                "evidence_message_ids": ["msg-1"],
                "evidence_quote": "I live in Edmonton now"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let memory = store
        .get_memory(&MemoryId("memory-contradicted-target".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.truth_status, TruthStatus::Denied);
    assert!(memory.contradiction_group.is_some());
    assert_eq!(memory.evidence_message_ids, vec!["msg-1".to_string()]);
    assert_eq!(memory.evidence["operation"], "contradict");
    assert_eq!(memory.evidence["reason"], "Sam corrected the location");

    let mutations = store
        .memory_mutations_for_memory(&MemoryId("memory-contradicted-target".into()), 10)
        .await
        .unwrap();
    let fields = mutations[0].data["fields"].as_array().unwrap();
    assert!(fields.contains(&json!("truth_status")));
    assert!(fields.contains(&json!("contradiction_group")));
    assert!(fields.contains(&json!("evidence")));
}
#[tokio::test]
async fn apply_review_can_supersede_existing_memory_with_replacement() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "apply_review")
        .expect("apply_review tool exists");
    assert!(
        tool.parameters["properties"]["memories"]["items"]["properties"]["operation"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("supersede"))
    );

    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    store
        .store_memory(&Memory {
            id: MemoryId("memory-old-location".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Fact,
            truth_status: TruthStatus::Stated,
            content: "Sam lives in Toronto.".into(),
            confidence: 0.8,
            evidence_message_ids: vec!["msg-old".into()],
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            ..Memory::default()
        })
        .await
        .unwrap();
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "supersede",
                "memory_id": "memory-old-location",
                "content": "Sam lives in Edmonton now.",
                "kind": "semantic",
                "memory_type": "fact",
                "truth_status": "confirmed",
                "confidence": 0.9,
                "importance": 0.7,
                "reason": "Sam corrected the old location",
                "evidence_message_ids": ["msg-1"],
                "evidence_quote": "I live in Edmonton now"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let replacement_id = state.memories_formed.last().unwrap().clone();
    assert_ne!(replacement_id, MemoryId("memory-old-location".into()));
    let replacement = store.get_memory(&replacement_id).await.unwrap().unwrap();
    assert_eq!(replacement.content, "Sam lives in Edmonton now.");
    assert_eq!(replacement.truth_status, TruthStatus::Confirmed);
    assert_eq!(
        replacement.supersedes.as_ref().map(|id| id.0.as_str()),
        Some("memory-old-location")
    );
    assert_eq!(replacement.subjects.len(), 1);
    assert_eq!(
        replacement.subjects[0].subject_type,
        MemorySubjectType::Profile
    );
    assert_eq!(replacement.subjects[0].subject_id, "profile-sam");
    assert_eq!(replacement.evidence["operation"], "supersede");
    assert_eq!(
        replacement.evidence["reason"],
        "Sam corrected the old location"
    );

    let old_memory = store
        .get_memory(&MemoryId("memory-old-location".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old_memory.truth_status, TruthStatus::Outdated);
    assert_eq!(old_memory.superseded_by, Some(replacement_id.clone()));
    assert_eq!(old_memory.evidence["operation"], "superseded");
    assert_eq!(
        old_memory.evidence["reason"],
        "Sam corrected the old location"
    );

    let mutations = store
        .memory_mutations_for_memory(&MemoryId("memory-old-location".into()), 10)
        .await
        .unwrap();
    assert_eq!(mutations[0].operation, "update");
    let fields = mutations[0].data["fields"].as_array().unwrap();
    assert!(fields.contains(&json!("truth_status")));
    assert!(fields.contains(&json!("superseded_by")));
    assert!(fields.contains(&json!("evidence")));
}
