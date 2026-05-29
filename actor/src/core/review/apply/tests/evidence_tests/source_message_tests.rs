use super::*;

#[tokio::test]
async fn apply_review_uses_cited_evidence_message_as_memory_source() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let other_profile = ProfileId("profile-alice".into());
    let other_person = PersonId("person-alice".into());
    let conversation = ConversationId("relay:local".into());
    store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Sam".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    store
        .add_person(&Person {
            id: person.clone(),
            name: Some("Sam".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
        .await
        .unwrap();
    let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
    ctx.router = Arc::new(router_with_successful_embedding_endpoint());
    let mut second = inbound(&other_profile, &other_person, &conversation);
    second.message_id = "msg-2".into();
    second.sender = Some(protocol::ObservedSender::primary(
        "relay", "alice", None, "test",
    ));
    second.channel = protocol::ChannelKey::new("relay", "alice", protocol::ChannelKind::Direct);
    second.content = "Alice prefers release notes with chosen_people.".into();
    ctx.messages.push(second);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "create",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Alice prefers release notes with chosen_people.",
                "evidence_message_ids": ["msg-2"],
                "source_spans": [{
                    "message_id": "msg-2",
                    "start_char": 0,
                    "end_char": 47,
                    "quote": "Alice prefers release notes with chosen_people."
                }]
            }],
            "social_relations": [{
                "person_a": "person-sam",
                "person_b": "person-alice",
                "relation": "coworker",
                "direction": "bidirectional",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated",
                "evidence_message_ids": ["msg-2"],
                "evidence_quote": "Alice prefers release notes with chosen_people.",
                "evidence": {"reason": "Alice stated the preference"}
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["memories"], 1);
    assert_eq!(parsed["social_relations"], 1);

    let memory = store
        .get_memory(&state.memories_formed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, "profile-alice");
    assert_eq!(memory.evidence["source_spans"][0]["message_id"], "msg-2");
    assert_eq!(
        memory.evidence["source_spans"][0]["quote"],
        "Alice prefers release notes with chosen_people."
    );
    assert_eq!(memory.embedding_model.as_deref(), Some("embed-review"));
    assert_eq!(memory.embedding.as_deref(), Some(&[0.1, 0.2, 0.3, 0.4][..]));
    match memory.source {
        MemorySource::Conversation {
            profile_id,
            person_id,
            message_id,
            ..
        } => {
            assert_eq!(profile_id, Some(other_profile));
            assert_eq!(person_id, Some(other_person.clone()));
            assert_eq!(message_id.as_deref(), Some("msg-2"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }

    let relations = store
        .get_relations(&PersonId("person-sam".into()))
        .await
        .unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].direction.as_str(), "bidirectional");
    let evidence = relations[0].evidence.as_ref().unwrap();
    assert_eq!(evidence["message_ids"].as_array().unwrap().len(), 1);
    assert_eq!(evidence["message_ids"][0], "msg-2");
    assert_eq!(relations[0].asserted_by.as_ref(), Some(&other_person));
    assert_eq!(
        evidence["quote"],
        "Alice prefers release notes with chosen_people."
    );
    assert_eq!(
        evidence["evidence"]["reason"],
        "Alice stated the preference"
    );
}
#[tokio::test]
async fn apply_review_skips_memory_with_unavailable_evidence_message_id() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "create",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Sam prefers concise release notes.",
                "evidence_message_ids": ["msg-missing"],
                "dedupe_key": "review:test:missing-evidence-memory"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;

    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["memories"], 0);
    assert!(state.memories_formed.is_empty());
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("unavailable evidence message ids"))
    }));
    let memories = store
        .recall(&crate::store::RecallQuery::by_text(
            "Sam prefers concise release notes.",
            5,
        ))
        .await
        .unwrap();
    assert!(memories.is_empty());
}
#[tokio::test]
async fn apply_review_skips_social_relation_with_unavailable_evidence_message_id() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let now = util::now();
    store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Sam".into()),
            summary: None,
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
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "social_relations": [{
                "person_a": person.0,
                "person_b": "person-alice",
                "relation": "coworker",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated",
                "evidence_message_ids": ["msg-missing"],
                "evidence_quote": "Sam said Alice is my coworker"
            }]
        }),
        &ctx,
        &mut state,
    )
    .await;

    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["social_relations"], 0);
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("unavailable evidence message ids"))
    }));
    let relations = store.get_relations(&person).await.unwrap();
    assert!(relations.is_empty());
}
