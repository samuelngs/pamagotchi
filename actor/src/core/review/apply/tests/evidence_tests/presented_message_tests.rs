use super::*;

#[tokio::test]
async fn apply_review_uses_presented_injected_message_evidence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let source_profile = ProfileId("profile-source".into());
    let source_person = PersonId("person-source".into());
    let injected_profile = ProfileId("profile-injected".into());
    let injected_person = PersonId("person-injected".into());
    let source_conversation = ConversationId("relay:source".into());
    let injected_conversation = ConversationId("relay:injected".into());
    let now = util::now();
    for (profile, person, name) in [
        (&source_profile, &source_person, "Source"),
        (&injected_profile, &injected_person, "Injected"),
    ] {
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some(name.into()),
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
                name: Some(name.into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(profile, person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
    }
    let (mut ctx, mut session_state) = test_context(
        store.clone(),
        &source_profile,
        &source_person,
        &source_conversation,
    );
    ctx.action_id = ActionId("review-injected-evidence".into());
    ctx.conversation = None;
    let mut injected = inbound(&injected_profile, &injected_person, &injected_conversation);
    injected.message_id = "msg-injected".into();
    injected.content = "Injected says release notes need chosen_people and rollback paths.".into();
    injected.timestamp = 1001;
    session_state.presented_injected_messages.push(injected);

    let result = apply(
        &json!({
            "profile_updates": [{
                "profile_id": injected_profile.0.clone(),
                "summary": "Injected profile wants release notes with chosen_people and rollback paths.",
                "evidence_message_ids": ["msg-injected"]
            }],
            "person_updates": [{
                "person_id": injected_person.0.clone(),
                "summary": "Injected person wants release notes with chosen_people and rollback paths.",
                "evidence_message_ids": ["msg-injected"]
            }],
            "memories": [{
                "operation": "upsert",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Injected person prefers release notes with chosen_people and rollback paths.",
                "evidence_message_ids": ["msg-injected"],
                "dedupe_key": "preference:profile-injected:release-note-chosen_human-rollback"
            }],
            "relationship_delta": [{
                "person_id": injected_person.0.clone(),
                "familiarity_delta": 0.05,
                "reason": "injected message was presented during review"
            }],
            "social_relations": [{
                "person_a": source_person.0.clone(),
                "person_b": injected_person.0.clone(),
                "relation": "coworker",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated",
                "evidence_message_ids": ["msg-injected"]
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["profile_updates"], 1);
    assert_eq!(parsed["person_updates"], 1);
    assert_eq!(parsed["memories"], 1);
    assert_eq!(parsed["relationship_deltas"], 1);
    assert_eq!(parsed["social_relations"], 1);
    assert_eq!(parsed["skipped"].as_array().unwrap().len(), 0);

    let updated_profile = store.get_profile(&injected_profile).await.unwrap().unwrap();
    assert_eq!(
        updated_profile.summary.as_deref(),
        Some("Injected profile wants release notes with chosen_people and rollback paths.")
    );
    let updated_person = store.get_person(&injected_person).await.unwrap().unwrap();
    assert_eq!(
        updated_person.summary.as_deref(),
        Some("Injected person wants release notes with chosen_people and rollback paths.")
    );

    let memory = store
        .get_memory(&session_state.memories_formed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, injected_profile.0.as_str());
    match memory.source {
        MemorySource::Conversation {
            conversation_id,
            profile_id,
            person_id,
            message_id,
            ..
        } => {
            assert_eq!(conversation_id, injected_conversation);
            assert_eq!(profile_id, Some(injected_profile.clone()));
            assert_eq!(person_id, Some(injected_person.clone()));
            assert_eq!(message_id.as_deref(), Some("msg-injected"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }
    assert_eq!(
        session_state.delta.relationship_changes[0].person,
        injected_person.clone()
    );
    let relations = store.get_relations(&source_person).await.unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].person_b, injected_person);
    assert_eq!(
        relations[0].asserted_by.as_ref(),
        Some(&PersonId("person-injected".into()))
    );
    assert_eq!(
        relations[0].evidence.as_ref().unwrap()["message_ids"][0],
        "msg-injected"
    );
}
#[tokio::test]
async fn apply_review_uses_presented_read_message_evidence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let source_profile = ProfileId("profile-source".into());
    let source_person = PersonId("person-source".into());
    let read_profile = ProfileId("profile-read".into());
    let read_person = PersonId("person-read".into());
    let source_conversation = ConversationId("relay:source".into());
    let read_conversation = ConversationId("relay:read".into());
    let now = util::now();
    for (profile, person, name) in [
        (&source_profile, &source_person, "Source"),
        (&read_profile, &read_person, "Read"),
    ] {
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some(name.into()),
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
                name: Some(name.into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(profile, person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
    }
    let (mut ctx, mut session_state) = test_context(
        store.clone(),
        &source_profile,
        &source_person,
        &source_conversation,
    );
    ctx.action_id = ActionId("review-read-evidence".into());
    let mut read = inbound(&read_profile, &read_person, &read_conversation);
    read.message_id = "msg-read".into();
    read.content = "Read person prefers concise incident notes.".into();
    read.timestamp = 1001;
    session_state.presented_read_messages.push(read);

    let result = apply(
        &json!({
            "memories": [{
                "operation": "upsert",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Read person prefers concise incident notes.",
                "evidence_message_ids": ["msg-read"],
                "dedupe_key": "preference:profile-read:concise-incident-notes"
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["memories"], 1);
    let memories = store
        .recall(&RecallQuery::by_text("concise incident notes", 5))
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);
    let memory = &memories[0];
    assert_eq!(memory.evidence_message_ids, vec!["msg-read"]);
    assert_eq!(memory.subjects.len(), 1);
    assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory.subjects[0].subject_id, read_profile.0);
    match &memory.source {
        MemorySource::Conversation { message_id, .. } => {
            assert_eq!(message_id.as_deref(), Some("msg-read"));
        }
        other => panic!("expected conversation source, got {other:?}"),
    }
}
