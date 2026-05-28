use super::*;

#[tokio::test]
async fn apply_review_default_upsert_reuses_memory_across_review_actions() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let review_args = json!({
        "memories": [{
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "content": "Sam prefers concise future summaries.",
            "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
            "importance": 0.8,
            "confidence": 0.9,
            "evidence_message_ids": ["msg-1"]
        }]
    });

    let (mut first_ctx, mut first_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    first_ctx.action_id = ActionId("review-action-1".into());
    first_ctx.cancelled_note = Some("Post-turn review for action source-action-1".into());
    let first_result = apply(&review_args, &first_ctx, &mut first_state).await;
    let first: Value = serde_json::from_str(&first_result).unwrap();
    assert_eq!(first["status"], "applied");
    assert_eq!(first["memories"], 1);
    let first_memory_id = first_state.memories_formed[0].clone();
    let first_metrics = first_ctx.metrics.snapshot();
    assert_eq!(first_metrics.memory_created, 1);
    assert_eq!(first_metrics.memory_updated, 0);

    let (mut second_ctx, mut second_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    second_ctx.action_id = ActionId("review-action-2".into());
    second_ctx.cancelled_note = Some("Post-turn review for action source-action-2".into());
    let second_result = apply(&review_args, &second_ctx, &mut second_state).await;
    let second: Value = serde_json::from_str(&second_result).unwrap();
    assert_eq!(second["status"], "applied");
    assert_eq!(second["memories"], 1);
    assert_eq!(second_state.memories_formed[0], first_memory_id);
    let second_metrics = second_ctx.metrics.snapshot();
    assert_eq!(second_metrics.memory_created, 0);
    assert_eq!(second_metrics.memory_updated, 1);

    let memories = store
        .recall(&crate::store::RecallQuery::by_text(
            "concise future summaries",
            10,
        ))
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);
    let dedupe_key = memories[0].dedupe_key.as_deref().unwrap();
    assert!(dedupe_key.starts_with("review:memory:upsert:semantic:preference:stated:"));
    assert!(!dedupe_key.contains("review-action-1"));
    assert!(!dedupe_key.contains("review-action-2"));
    assert_eq!(
        store
            .review_outputs_for_action("review-action-1")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .review_outputs_for_action("review-action-2")
            .await
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn apply_review_persists_memory_without_embedding_when_embedding_endpoint_fails() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
    ctx.router = Arc::new(router_with_failing_embedding_endpoint());

    let review_args = json!({
        "memories": [{
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "content": "Sam prefers concise launch briefs.",
            "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
            "importance": 0.8,
            "confidence": 0.9,
            "evidence_message_ids": ["msg-1"]
        }]
    });

    let result = apply(&review_args, &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["memories"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let memory = store
        .get_memory(&state.memories_formed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.content, "Sam prefers concise launch briefs.");
    assert_eq!(memory.memory_type, MemoryType::Preference);
    assert!(memory.embedding.is_none());
}
