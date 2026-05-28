use super::*;

#[tokio::test]
async fn apply_review_defaults_uncertain_and_emotional_memories_to_transient() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [
                {
                    "operation": "create",
                    "kind": "semantic",
                    "memory_type": "hypothesis",
                    "stability": "stable",
                    "content": "Sam might be joking about moving to Mars.",
                    "evidence_message_ids": ["msg-1"],
                    "dedupe_key": "review:test:hypothesis-mars"
                },
                {
                    "operation": "create",
                    "kind": "episodic",
                    "memory_type": "emotional_state",
                    "truth_status": "stated",
                    "stability": "stable",
                    "content": "Sam feels annoyed about launch today.",
                    "evidence_message_ids": ["msg-1"],
                    "dedupe_key": "review:test:emotion-launch"
                }
            ]
        }),
        &ctx,
        &mut state,
    )
    .await;

    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["memories"], 2);
    assert_eq!(state.memories_formed.len(), 2);

    let hypothesis = store
        .get_memory(&state.memories_formed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hypothesis.memory_type, MemoryType::Hypothesis);
    assert_eq!(hypothesis.truth_status, TruthStatus::Inferred);
    assert_eq!(hypothesis.stability, MemoryStability::Transient);

    let emotion = store
        .get_memory(&state.memories_formed[1])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(emotion.memory_type, MemoryType::EmotionalState);
    assert_eq!(emotion.truth_status, TruthStatus::Stated);
    assert_eq!(emotion.stability, MemoryStability::Transient);
}
#[tokio::test]
async fn apply_review_skips_memory_subjects_outside_evidence_profile_or_identity() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
    ctx.messages[0].identity = Some(IdentityId("identity-sam".into()));

    let result = apply(
        &json!({
            "memories": [
                {
                    "operation": "create",
                    "content": "Sam prefers concise launch notes.",
                    "subjects": [{
                        "type": "profile",
                        "id": "profile-sam",
                        "role": "about",
                        "confidence": 1.0
                    }],
                    "evidence_message_ids": ["msg-1"],
                    "dedupe_key": "review:test:allowed-profile-subject"
                },
                {
                    "operation": "create",
                    "content": "Alice prefers private escalation.",
                    "subjects": [{
                        "type": "profile",
                        "id": "profile-alice",
                        "role": "about",
                        "confidence": 1.0
                    }],
                    "evidence_message_ids": ["msg-1"],
                    "dedupe_key": "review:test:blocked-profile-subject"
                },
                {
                    "operation": "create",
                    "content": "Sam's relay identity is the current speaker.",
                    "subjects": [{
                        "type": "identity",
                        "id": "identity-sam",
                        "role": "about",
                        "confidence": 1.0
                    }],
                    "evidence_message_ids": ["msg-1"],
                    "dedupe_key": "review:test:allowed-identity-subject"
                },
                {
                    "operation": "create",
                    "content": "Alice's identity prefers SMS.",
                    "subjects": [{
                        "type": "identity",
                        "id": "identity-alice",
                        "role": "about",
                        "confidence": 1.0
                    }],
                    "evidence_message_ids": ["msg-1"],
                    "dedupe_key": "review:test:blocked-identity-subject"
                }
            ]
        }),
        &ctx,
        &mut state,
    )
    .await;

    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["memories"], 2);
    assert_eq!(state.memories_formed.len(), 2);
    let skipped = parsed["skipped"].as_array().unwrap();
    assert_eq!(
        skipped
            .iter()
            .filter(|item| item
                .as_str()
                .is_some_and(|message| message.contains("outside review evidence")))
            .count(),
        2
    );

    let mut subject_ids = Vec::new();
    for memory_id in &state.memories_formed {
        let memory = store.get_memory(memory_id).await.unwrap().unwrap();
        subject_ids.extend(
            memory
                .subjects
                .iter()
                .map(|subject| (subject.subject_type.clone(), subject.subject_id.clone())),
        );
    }
    assert!(subject_ids.contains(&(MemorySubjectType::Profile, "profile-sam".into())));
    assert!(subject_ids.contains(&(MemorySubjectType::Identity, "identity-sam".into())));
    assert!(!subject_ids.contains(&(MemorySubjectType::Profile, "profile-alice".into())));
    assert!(!subject_ids.contains(&(MemorySubjectType::Identity, "identity-alice".into())));
}
#[tokio::test]
async fn apply_review_derives_sensitive_memory_policy() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let review_args = json!({
        "memories": [{
            "kind": "semantic",
            "memory_type": "fact",
            "truth_status": "stated",
            "content": "Sam mentioned a private medical follow-up.",
            "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
            "importance": 0.7,
            "confidence": 0.9,
            "sensitivity": 0.2,
            "sensitivity_category": "medical",
            "evidence_message_ids": ["msg-1"]
        }]
    });

    let result = apply(&review_args, &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);

    let memory = store
        .get_memory(&state.memories_formed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.privacy_category, PrivacyCategory::Sensitive);
    assert_eq!(memory.visibility_scope, VisibilityScope::Profile);
    assert!(memory.next_review_at.is_some());
}
