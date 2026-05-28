use super::*;

#[tokio::test]
async fn apply_review_writes_structured_outputs() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let now = util::now();
    store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Sam".into()),
            summary: Some("Sam likes concise summaries and deployment updates.".into()),
            comm_style: None,
            first_seen: now,
            last_seen: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .add_person(&Person {
            id: person.clone(),
            name: Some("Sam".into()),
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
        .await
        .unwrap();
    store
        .append_message(
            &conversation,
            Some("relay"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "make future summaries concise".into(),
                identity: None,
                profile: Some(profile.clone()),
                person: Some(person.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "source-action".into(),
            kind: "respond".into(),
            task: "Respond to Sam".into(),
            conversation: Some(conversation.clone()),
            started_at: now - 20,
            ended_at: None,
            status: "running".into(),
            responded: false,
            attempts: 0,
        })
        .await
        .unwrap();
    store
        .finish_action_run(
            "source-action",
            now - 10,
            "completed",
            true,
            1,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    store
        .store_memory(&Memory {
            id: MemoryId("old-summary-preference".into()),
            kind: MemoryKind::Semantic,
            memory_type: MemoryType::Preference,
            truth_status: TruthStatus::Stated,
            content: "Sam prefers long future summaries.".into(),
            source: MemorySource::Reflection,
            importance: 0.7,
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
    let (ctx, mut session_state) = test_context(store.clone(), &profile, &person, &conversation);

    let review_args = json!({
        "profile_updates": [{
            "profile_id": profile.0,
            "summary": "short",
            "comm_style": "Concise and practical."
        }],
        "person_updates": [{
            "person_id": person.0,
            "summary": "Sam prefers concise operational updates across contexts.",
            "comm_style": "Direct, brief, practical."
        }],
        "memories": [{
            "operation": "upsert",
            "kind": "semantic",
            "memory_type": "preference",
            "truth_status": "stated",
            "content": "Sam prefers concise future summaries.",
            "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
            "importance": 0.8,
            "confidence": 0.9,
            "evidence_message_ids": ["msg-1"],
            "supersedes": "old-summary-preference",
            "dedupe_key": "preference:profile-sam:concise-summaries"
        }],
        "relationship_delta": [{
            "person_id": person.0,
            "familiarity_delta": 0.5,
            "trust_delta": 0.5,
            "valence_delta": 0.5,
            "closeness_delta": 0.5,
            "reliability_delta": 0.5,
            "reciprocity_delta": 0.5,
            "conflict_delta": -0.5,
            "proactive_consent": "allowed",
            "response_cadence": "reply within one business day",
            "channel_preference": "Discord for quick deployment coordination",
            "reason": "brief friendly exchange"
        }],
        "social_relations": [{
            "person_a": "person-sam",
            "person_b": "person-alice",
            "relation": "coworker",
            "confidence": 0.8,
            "status": "stated",
            "source_kind": "stated",
            "evidence": {"quote": "Alice is my coworker"}
        }],
        "open_loops": [{
            "task": "Ask whether concise summaries helped",
            "fire_at": now + 3600,
            "conversation_id": conversation.0,
            "source_memory_id": "old-summary-preference",
            "dedupe_key": "review:test:followup"
        }, {
            "task": "Ask about the private medical update",
            "fire_at": now + 3600,
            "conversation_id": conversation.0,
            "sensitive": true,
            "dedupe_key": "review:test:sensitive-followup"
        }, {
            "task": "Ask Alice whether Sam's summary preference applies to her too",
            "fire_at": now + 3600,
            "person_id": "person-alice",
            "dedupe_key": "review:test:third-party-followup"
        }],
        "conversation_summary": {
            "conversation_id": conversation.0,
            "summary": "Sam asked for concise future summaries.",
            "covered_message_ids": ["msg-1"]
        }
    });

    let result = apply(&review_args, &ctx, &mut session_state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["profile_updates"], 1);
    assert_eq!(parsed["person_updates"], 1);
    assert_eq!(parsed["memories"], 1);
    assert_eq!(parsed["relationship_deltas"], 1);
    assert_eq!(parsed["social_relations"], 1);
    assert_eq!(parsed["open_loops"], 1);
    assert_eq!(parsed["conversation_summaries"], 1);
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("requires chosen-person approval"))
    }));
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("unverified third-party"))
    }));
    let review_outputs = store
        .review_outputs_for_action("review-action")
        .await
        .unwrap();
    assert_eq!(review_outputs.len(), 1);
    assert_eq!(
        review_outputs[0].source_action_id.as_deref(),
        Some("source-action")
    );
    let metrics = ctx.metrics.snapshot();
    assert_eq!(metrics.review_outputs, 1);
    assert!(metrics.review_latency_ms_total >= 10_000);
    assert_eq!(
        review_outputs[0].input["memories"][0]["dedupe_key"],
        "preference:profile-sam:concise-summaries"
    );
    assert_eq!(
        review_outputs[0].input["memories"][0]["content"],
        "[redacted]"
    );
    assert_eq!(
        review_outputs[0].input["profile_updates"][0]["summary"],
        "[redacted]"
    );
    assert_eq!(
        review_outputs[0].input["conversation_summary"]["summary"],
        "[redacted]"
    );
    assert_eq!(
        review_outputs[0].input["relationship_delta"][0]["response_cadence"],
        "[redacted]"
    );
    assert_eq!(
        review_outputs[0].input["relationship_delta"][0]["channel_preference"],
        "[redacted]"
    );
    assert_eq!(review_outputs[0].result["memories"], 1);

    let updated_profile = store.get_profile(&profile).await.unwrap().unwrap();
    assert_eq!(
        updated_profile.summary.as_deref(),
        Some("Sam likes concise summaries and deployment updates.")
    );
    assert_eq!(
        updated_profile.comm_style.as_deref(),
        Some("Concise and practical.")
    );
    let updated_person = store.get_person(&person).await.unwrap().unwrap();
    assert_eq!(
        updated_person.summary.as_deref(),
        Some("Sam prefers concise operational updates across contexts.")
    );
    assert_eq!(
        updated_person.comm_style.as_deref(),
        Some("Direct, brief, practical.")
    );

    let memories = store
        .recall(&crate::store::RecallQuery::by_text("concise summaries", 5))
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(
        memories[0].dedupe_key.as_deref(),
        Some("preference:profile-sam:concise-summaries")
    );
    assert_eq!(
        memories[0].supersedes.as_ref().map(|id| id.0.as_str()),
        Some("old-summary-preference")
    );
    let old_memory = store
        .get_memory(&MemoryId("old-summary-preference".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old_memory.truth_status, TruthStatus::Outdated);
    assert_eq!(
        old_memory.superseded_by,
        Some(session_state.memories_formed[0].clone())
    );
    assert_eq!(session_state.memories_formed.len(), 1);
    assert_eq!(
        session_state.delta.relationship_changes[0].trust_delta,
        0.05
    );
    assert_eq!(
        session_state.delta.relationship_changes[0].familiarity_delta,
        0.1
    );
    assert_eq!(
        session_state.delta.relationship_changes[0].valence_delta,
        0.1
    );
    assert_eq!(
        session_state.delta.relationship_changes[0].proactive_consent,
        Some(ProactiveConsent::Allowed)
    );
    assert_eq!(
        session_state.delta.relationship_changes[0]
            .response_cadence
            .as_deref(),
        Some("reply within one business day")
    );
    assert_eq!(
        session_state.delta.relationship_changes[0]
            .channel_preference
            .as_deref(),
        Some("Discord for quick deployment coordination")
    );
    assert_eq!(session_state.delta.relationship_signal_updates.len(), 1);
    assert_eq!(
        session_state.delta.relationship_signal_updates[0].closeness_delta,
        0.05
    );
    assert_eq!(
        session_state.delta.relationship_signal_updates[0].reliability_delta,
        0.05
    );
    assert_eq!(
        session_state.delta.relationship_signal_updates[0].reciprocity_delta,
        0.05
    );
    assert_eq!(
        session_state.delta.relationship_signal_updates[0].conflict_delta,
        -0.05
    );
    let due_intents = store.due_intents(now + 3600, 10).await.unwrap();
    assert_eq!(due_intents.len(), 1);
    assert_eq!(
        due_intents[0]
            .source_memory
            .as_ref()
            .map(|id| id.0.as_str()),
        Some("old-summary-preference")
    );
    let relations = store.get_relations(&person).await.unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].person_b, PersonId("person-alice".into()));
    assert_eq!(relations[0].relation.as_str(), "coworker");
    assert_eq!(relations[0].confidence, 0.8);
    assert_eq!(relations[0].status, RelationStatus::Stated);
    assert_eq!(relations[0].source_kind, RelationSource::Stated);
    assert_eq!(relations[0].asserted_by.as_ref(), Some(&person));
    assert_eq!(
        relations[0].evidence.as_ref().unwrap()["message_ids"][0],
        "msg-1"
    );
    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(
        conversations[0].summary.as_deref(),
        Some("Sam asked for concise future summaries.")
    );
    assert_eq!(conversations[0].summary_version, 1);

    let (retry_ctx, mut retry_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    let duplicate_result = apply(&review_args, &retry_ctx, &mut retry_state).await;
    let duplicate: Value = serde_json::from_str(&duplicate_result).unwrap();
    assert_eq!(duplicate["status"], "already_applied");
    assert_eq!(duplicate["memories"], 1);
    assert_eq!(duplicate["relationship_deltas"], 1);
    assert_eq!(duplicate["social_relations"], 1);
    assert_eq!(duplicate["open_loops"], 1);
    assert_eq!(session_state.memories_formed.len(), 1);
    assert_eq!(session_state.delta.relationship_changes.len(), 1);
    assert_eq!(retry_state.memories_formed.len(), 0);
    assert_eq!(retry_state.delta.relationship_changes.len(), 0);
    assert_eq!(store.due_intents(now + 3600, 10).await.unwrap().len(), 1);
    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(conversations[0].summary_version, 1);
    let review_outputs = store
        .review_outputs_for_action("review-action")
        .await
        .unwrap();
    assert_eq!(review_outputs.len(), 1);
    assert_eq!(review_outputs[0].result["status"], "applied");
    let review_outputs_for_source = store
        .review_outputs_for_source_action("source-action")
        .await
        .unwrap();
    assert_eq!(review_outputs_for_source.len(), 1);

    let (mut duplicate_source_ctx, mut duplicate_source_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    duplicate_source_ctx.action_id = ActionId("review-action-duplicate-source".into());
    duplicate_source_ctx.cancelled_note = Some("Post-turn review for action source-action".into());
    let duplicate_source_result = apply(
        &review_args,
        &duplicate_source_ctx,
        &mut duplicate_source_state,
    )
    .await;
    let duplicate_source: Value = serde_json::from_str(&duplicate_source_result).unwrap();
    assert_eq!(duplicate_source["status"], "already_applied");
    assert_eq!(duplicate_source["memories"], 1);
    assert_eq!(duplicate_source_state.memories_formed.len(), 0);
    assert_eq!(duplicate_source_state.delta.relationship_changes.len(), 0);
    assert_eq!(store.due_intents(now + 3600, 10).await.unwrap().len(), 1);
    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(conversations[0].summary_version, 1);
    assert_eq!(
        store
            .review_outputs_for_action("review-action-duplicate-source")
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store
            .review_outputs_for_source_action("source-action")
            .await
            .unwrap()
            .len(),
        1
    );

    let (mut summary_ctx, mut summary_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    summary_ctx.action_id = ActionId("review-action-summary-merge".into());
    summary_ctx.cancelled_note =
        Some("Post-turn review for action source-action-summary-merge".into());
    let summary_args = json!({
        "conversation_summary": {
            "conversation_id": conversation.0,
            "summary": "Uses checklist.",
            "covered_message_ids": ["msg-1", "msg-2"]
        }
    });
    let summary_result = apply(&summary_args, &summary_ctx, &mut summary_state).await;
    let summary_parsed: Value = serde_json::from_str(&summary_result).unwrap();
    assert_eq!(summary_parsed["conversation_summaries"], 1);
    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(
        conversations[0].summary.as_deref(),
        Some("Sam asked for concise future summaries. Uses checklist.")
    );
    assert_eq!(
        conversations[0].summary_covered_message_ids,
        vec!["msg-1".to_string(), "msg-2".to_string()]
    );
    assert_eq!(conversations[0].summary_version, 2);

    let (mut redundant_ctx, mut redundant_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    redundant_ctx.action_id = ActionId("review-action-summary-redundant".into());
    redundant_ctx.cancelled_note =
        Some("Post-turn review for action source-action-summary-redundant".into());
    let redundant_args = json!({
        "conversation_summary": {
            "conversation_id": conversation.0,
            "summary": "Sam asked for concise future summaries.",
            "covered_message_ids": ["msg-1", "msg-2"]
        }
    });
    let redundant_result = apply(&redundant_args, &redundant_ctx, &mut redundant_state).await;
    let redundant: Value = serde_json::from_str(&redundant_result).unwrap();
    assert_eq!(redundant["conversation_summaries"], 0);
    assert!(redundant["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("had no new fields"))
    }));
    let conversations = store.list_conversations().await.unwrap();
    assert_eq!(conversations[0].summary_version, 2);
}
