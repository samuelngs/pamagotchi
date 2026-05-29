use super::*;

#[test]
fn slow_query_threshold_is_inclusive() {
    assert!(!sqlite_query_is_slow(
        std::time::Duration::from_millis(99),
        std::time::Duration::from_millis(100),
    ));
    assert!(sqlite_query_is_slow(
        std::time::Duration::from_millis(100),
        std::time::Duration::from_millis(100),
    ));
}

#[tokio::test]
async fn debug_views_return_bounded_recent_records() {
    let store = test_store();
    store
        .add_profile(&sample_profile("profile-debug", "Debug User"))
        .await
        .unwrap();
    store
        .add_identity(&sample_identity(
            "identity-debug",
            "discord",
            "debug-user",
            "Debug User",
        ))
        .await
        .unwrap();
    store
        .add_person(&sample_person("person-debug", "Debug User"))
        .await
        .unwrap();
    store
        .link_identity_to_profile(
            &IdentityId("identity-debug".into()),
            &ProfileId("profile-debug".into()),
            0.9,
            Some(&serde_json::json!({"message_id": "msg-link-profile"})),
        )
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &ProfileId("profile-debug".into()),
            &PersonId("person-debug".into()),
            PersonProfileStatus::Verified,
            0.95,
            Some(&serde_json::json!({"message_id": "msg-link-person"})),
        )
        .await
        .unwrap();
    store
        .add_group(&Group {
            id: GroupId("group-debug".into()),
            name: "Debug Group".into(),
            gateway_id: "discord".into(),
            external_id: "debug-channel".into(),
            context: GroupContext::Work,
            members: vec![PersonId("person-debug".into())],
        })
        .await
        .unwrap();
    let mut memory = sample_memory(
        "debug-memory",
        "Debug memory content",
        vec![0.1, 0.2, 0.3, 0.4],
    );
    memory.created_at = 2000;
    memory.subjects = vec![MemorySubject::profile(
        ProfileId("profile-debug".into()),
        Some("about".into()),
        1.0,
    )];
    store.store_memory(&memory).await.unwrap();

    store
        .create_intent(&IntentRecord {
            id: "intent-debug".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Inspect debug snapshot".into(),
            person: None,
            profile: Some(ProfileId("profile-debug".into())),
            conversation: None,
            fire_at: Some(3000),
            condition: None,
            recurrence: None,
            priority: 80,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1000,
            updated_at: 1000,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-pending-approval-debug".into(),
            kind: "scheduled".into(),
            status: "pending_approval".into(),
            task: "Inspect pending approval debug snapshot".into(),
            person: None,
            profile: Some(ProfileId("profile-debug".into())),
            conversation: None,
            fire_at: Some(3001),
            condition: None,
            recurrence: None,
            priority: 70,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1001,
            updated_at: 1001,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "action-debug".into(),
            kind: "respond".into(),
            task: "Debug action".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 4000,
            ended_at: Some(4001),
            status: "completed".into(),
            responded: true,
            attempts: 1,
        })
        .await
        .unwrap();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "review-action-debug".into(),
            kind: "review".into(),
            task: "Review debug action".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 3999,
            ended_at: Some(4002),
            status: "completed".into(),
            responded: false,
            attempts: 1,
        })
        .await
        .unwrap();
    store
        .mark_review_scheduled("action-debug", "review-action-debug", 4001)
        .await
        .unwrap();
    store
        .record_review_output(&ReviewOutputAudit {
            id: "review-debug".into(),
            review_action_id: "review-action-debug".into(),
            source_action_id: Some("action-debug".into()),
            input: serde_json::json!({"conversation_summary": {"summary": "debug"}}),
            result: serde_json::json!({"conversation_summary": 1}),
            applied_at: 4002,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-failed-old".into(),
            kind: "message".into(),
            payload: serde_json::json!({"content": "older private payload"}),
            status: "pending".into(),
            due_at: 3900,
            attempts: 0,
            dedupe_key: Some("event-failed-old".into()),
            created_at: 3900,
            updated_at: 3900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-failed-new".into(),
            kind: "message".into(),
            payload: serde_json::json!({"content": "newer private payload"}),
            status: "pending".into(),
            due_at: 3901,
            attempts: 0,
            dedupe_key: Some("event-failed-new".into()),
            created_at: 3901,
            updated_at: 3901,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .mark_event_failed("event-failed-old", 4003, Some("older malformed payload"))
        .await
        .unwrap();
    store
        .mark_event_failed("event-failed-new", 4004, Some("newer malformed payload"))
        .await
        .unwrap();

    let profiles = store.list_profiles().await.unwrap();
    assert_eq!(profiles[0].id.0, "profile-debug");

    let memories = store.debug_recent_memories(1).await.unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].id.0, "debug-memory");
    assert_eq!(memories[0].subjects[0].subject_id, "profile-debug");
    let memory_subjects = store.debug_memory_subjects(1).await.unwrap();
    assert_eq!(memory_subjects.len(), 1);
    assert_eq!(memory_subjects[0].subject_type, MemorySubjectType::Profile);
    assert_eq!(memory_subjects[0].subject_id, "profile-debug");
    assert_eq!(memory_subjects[0].memory_count, 1);
    assert_eq!(memory_subjects[0].latest_memory_ids[0].0, "debug-memory");
    let profile_links = store.debug_profile_identity_links(1).await.unwrap();
    assert_eq!(profile_links.len(), 1);
    assert_eq!(profile_links[0].profile_id.0, "profile-debug");
    assert_eq!(profile_links[0].identity_id.0, "identity-debug");
    assert_eq!(
        profile_links[0].evidence.as_ref().unwrap()["message_id"],
        "msg-link-profile"
    );
    let person_links = store.debug_person_profile_links(1).await.unwrap();
    assert_eq!(person_links.len(), 1);
    assert_eq!(person_links[0].person_id.0, "person-debug");
    assert_eq!(person_links[0].profile_id.0, "profile-debug");
    assert_eq!(
        person_links[0].evidence.as_ref().unwrap()["message_id"],
        "msg-link-person"
    );
    let groups = store.debug_groups(1).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id.0, "group-debug");
    assert_eq!(groups[0].members, vec![PersonId("person-debug".into())]);

    let intents = store.debug_active_intents(1).await.unwrap();
    assert_eq!(intents[0].id, "intent-debug");
    let intents = store.debug_active_intents(10).await.unwrap();
    assert!(intents.iter().any(|intent| intent.id == "intent-debug"));
    assert!(
        intents
            .iter()
            .any(|intent| intent.id == "intent-pending-approval-debug"
                && intent.status == "pending_approval")
    );

    let actions = store.debug_recent_action_runs(1).await.unwrap();
    assert_eq!(actions[0].action_id, "action-debug");

    let reviews = store.debug_recent_review_outputs(1).await.unwrap();
    assert_eq!(reviews[0].id, "review-debug");
    assert_eq!(reviews[0].source_action_id.as_deref(), Some("action-debug"));

    let review_jobs = store.debug_recent_review_jobs(1).await.unwrap();
    assert_eq!(review_jobs.len(), 1);
    assert_eq!(review_jobs[0].source_action_id, "action-debug");
    assert_eq!(review_jobs[0].review_action_id, "review-action-debug");
    assert_eq!(review_jobs[0].source_kind.as_deref(), Some("respond"));
    assert_eq!(review_jobs[0].source_status.as_deref(), Some("completed"));
    assert_eq!(review_jobs[0].review_status.as_deref(), Some("completed"));
    assert_eq!(review_jobs[0].output_count, 1);
    assert_eq!(review_jobs[0].last_applied_at, Some(4002));

    let mutations = store.debug_recent_memory_mutations(1).await.unwrap();
    assert_eq!(mutations.len(), 1);
    assert_eq!(mutations[0].memory.0, "debug-memory");
    assert_eq!(mutations[0].operation, "create");
    assert_eq!(mutations[0].data["input_memory_id"], "debug-memory");

    let failed_events = store.debug_recent_failed_events(1).await.unwrap();
    assert_eq!(failed_events.len(), 1);
    assert_eq!(failed_events[0].id, "event-failed-new");
    assert_eq!(failed_events[0].status, "failed");
    assert_eq!(failed_events[0].attempts, 1);
    assert_eq!(
        failed_events[0].last_error.as_deref(),
        Some("newer malformed payload")
    );
    let failed_event_json = serde_json::to_value(&failed_events[0]).unwrap();
    assert!(failed_event_json.get("payload").is_none());
}

#[tokio::test]
async fn fresh_schema_has_no_legacy_people_tables() {
    let store = test_store();
    let conn = store.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap();
    let tables = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect::<HashSet<_>>();

    assert!(tables.contains("persons"));
    assert!(tables.contains("memory_subjects"));
    assert!(!tables.contains("people"));
    assert!(!tables.contains("memory_people"));

    let thought_columns = conn
        .prepare("PRAGMA table_info(thoughts)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect::<HashSet<_>>();
    assert!(thought_columns.contains("subjects"));
    assert!(!thought_columns.contains("people"));
}
