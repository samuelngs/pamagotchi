use super::*;

#[tokio::test]
async fn debug_snapshot_includes_memory_mutations() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert!(snapshot["memory_mutations"].is_array());
}
#[tokio::test]
async fn debug_snapshot_includes_memory_subject_index() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    let profile = ProfileId("profile-debug".into());
    let mut memory = Memory {
        id: MemoryId("memory-subject-debug".into()),
        kind: MemoryKind::Semantic,
        content: "Debug profile prefers concise summaries.".into(),
        source: MemorySource::External,
        subjects: vec![MemorySubject::profile(
            profile.clone(),
            Some("about".into()),
            1.0,
        )],
        created_at: 1000,
        accessed_at: 1000,
        ..Memory::default()
    };
    memory.embedding = Some(vec![0.1, 0.2, 0.3, 0.4]);
    store.store_memory(&memory).await.unwrap();

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(snapshot["memory_subjects"][0]["subject_id"], profile.0);
    assert_eq!(snapshot["memory_subjects"][0]["memory_count"], 1);
    assert_eq!(
        snapshot["memory_subjects"][0]["latest_memory_ids"][0],
        "memory-subject-debug"
    );
}
#[tokio::test]
async fn debug_snapshot_includes_identity_link_evidence() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    let identity = Identity {
        id: IdentityId("identity-debug".into()),
        gateway_id: "discord".into(),
        external_id: "debug-user".into(),
        display_name: Some("Debug User".into()),
        metadata: None,
        created_at: 1000,
        last_seen_at: 1000,
    };
    let profile = Profile {
        id: ProfileId("profile-debug".into()),
        display_name: Some("Debug User".into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
        created_at: 1000,
        updated_at: 1000,
    };
    let person = Person {
        id: PersonId("person-debug".into()),
        name: Some("Debug User".into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
    };
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store.add_person(&person).await.unwrap();
    store
        .link_identity_to_profile(
            &identity.id,
            &profile.id,
            0.91,
            Some(&serde_json::json!({"message_id": "msg-profile-link"})),
        )
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            &person.id,
            PersonProfileStatus::Verified,
            0.97,
            Some(&serde_json::json!({"message_id": "msg-person-link"})),
        )
        .await
        .unwrap();

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(
        snapshot["profile_identity_links"][0]["evidence"]["message_id"],
        "msg-profile-link"
    );
    assert_eq!(
        snapshot["person_profile_links"][0]["evidence"]["message_id"],
        "msg-person-link"
    );
}
#[tokio::test]
async fn debug_snapshot_includes_groups_and_members() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    let person = Person {
        id: PersonId("person-group-debug".into()),
        name: Some("Group Member".into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
    };
    store.add_person(&person).await.unwrap();
    store
        .add_group(&Group {
            id: GroupId("group-debug".into()),
            name: "Debug Group".into(),
            gateway_id: "discord".into(),
            external_id: "debug-channel".into(),
            context: GroupContext::Social,
            members: vec![person.id.clone()],
        })
        .await
        .unwrap();

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(snapshot["groups"][0]["id"], "group-debug");
    assert_eq!(snapshot["groups"][0]["name"], "Debug Group");
    assert_eq!(snapshot["groups"][0]["members"][0], "person-group-debug");
}
#[tokio::test]
async fn debug_snapshot_includes_failed_events_without_payloads() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-failed-debug".into(),
            kind: "message".into(),
            payload: serde_json::json!({"content": "private message body"}),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: Some("event-failed-debug".into()),
            created_at: 1000,
            updated_at: 1000,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();
    store
        .mark_event_failed("event-failed-debug", 1001, Some("malformed payload"))
        .await
        .unwrap();

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(snapshot["failed_events"][0]["id"], "event-failed-debug");
    assert_eq!(snapshot["failed_events"][0]["kind"], "message");
    assert_eq!(
        snapshot["failed_events"][0]["last_error"],
        "malformed payload"
    );
    assert!(snapshot["failed_events"][0].get("payload").is_none());
}
#[tokio::test]
async fn debug_snapshot_includes_review_jobs() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "action-reviewed-debug".into(),
            kind: "respond".into(),
            task: "Respond before review".into(),
            conversation: Some(ConversationId("relay:local".into())),
            started_at: 1000,
            ended_at: Some(1001),
            status: "completed".into(),
            responded: true,
            attempts: 1,
        })
        .await
        .unwrap();
    store
        .mark_review_scheduled("action-reviewed-debug", "review-action-debug", 1002)
        .await
        .unwrap();

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(
        snapshot["review_jobs"][0]["source_action_id"],
        "action-reviewed-debug"
    );
    assert_eq!(
        snapshot["review_jobs"][0]["review_action_id"],
        "review-action-debug"
    );
    assert_eq!(snapshot["review_jobs"][0]["source_kind"], "respond");
}
#[tokio::test]
async fn debug_snapshot_includes_action_traces() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    store
        .start_action_run(&ActionRunRecord {
            action_id: "action-debug".into(),
            kind: "respond".into(),
            task: "Respond to message".into(),
            conversation: None,
            started_at: 1000,
            ended_at: None,
            status: "running".into(),
            responded: false,
            attempts: 0,
        })
        .await
        .unwrap();
    store
        .append_action_turn(&ActionTurnRecord {
            action_id: "action-debug".into(),
            turn: 0,
            attempt: 1,
            prompt_hash: "hash-debug".into(),
            model: Some("model-debug".into()),
            finish: Some("tool_calls".into()),
            input_tokens: Some(10),
            output_tokens: Some(5),
            text_len: 12,
            reasoning_len: 0,
            tool_call_count: 1,
            created_at: 1001,
        })
        .await
        .unwrap();
    store
        .append_tool_call(&ToolCallRecord {
            action_id: "action-debug".into(),
            turn: 0,
            call_id: "call-debug".into(),
            name: "send_message".into(),
            args: serde_json::json!({"content": "hello"}),
            result: serde_json::json!({"status": "sent"}),
            success: true,
            started_at: 1002,
            ended_at: 1003,
        })
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: "action-debug".into(),
            role: "assistant".into(),
            conversation: Some(ConversationId("relay:local".into())),
            source_gateway_id: Some("relay".into()),
            source_message_id: Some("msg-private".into()),
            sender_external_id: Some("sender-private".into()),
            reply_external_id: Some("reply-private".into()),
            content: Some("private reply body".into()),
            created_at: 1004,
        })
        .await
        .unwrap();
    store
        .append_outbound_delivery(&OutboundDeliveryRecord {
            action_id: "action-debug".into(),
            conversation: Some(ConversationId("relay:local".into())),
            message: None,
            channel: None,
            gateway_id: "relay".into(),
            external_id: "reply-private".into(),
            status: "failed".into(),
            error: Some("delivery failed for reply-private".into()),
            attempted_at: 1005,
        })
        .await
        .unwrap();

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(snapshot["action_runs"][0]["task"], "[redacted]");
    assert_eq!(
        snapshot["action_traces"][0]["run"]["action_id"],
        "action-debug"
    );
    assert_eq!(snapshot["action_traces"][0]["run"]["task"], "[redacted]");
    assert_eq!(
        snapshot["action_traces"][0]["turns"][0]["prompt_hash"],
        "hash-debug"
    );
    assert_eq!(
        snapshot["action_traces"][0]["tool_calls"][0]["name"],
        "send_message"
    );
    assert_eq!(
        snapshot["action_traces"][0]["tool_calls"][0]["args"]["content"],
        "[redacted]"
    );
    assert_eq!(
        snapshot["action_traces"][0]["messages"][0]["content"],
        "[redacted]"
    );
    assert_eq!(
        snapshot["action_traces"][0]["messages"][0]["source_message_id"],
        "[redacted]"
    );
    assert_eq!(
        snapshot["action_traces"][0]["messages"][0]["reply_external_id"],
        "[redacted]"
    );
    assert_eq!(
        snapshot["action_traces"][0]["deliveries"][0]["external_id"],
        "[redacted]"
    );
    assert_eq!(
        snapshot["action_traces"][0]["deliveries"][0]["error"],
        "[redacted]"
    );
}
#[tokio::test]
async fn debug_snapshot_includes_actor_metrics() {
    let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
    let metrics = ActorMetrics::default();
    metrics.record_event_received();
    metrics.record_tool_call("send_message", true);

    let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

    assert_eq!(snapshot["metrics"]["events_received"], 1);
    assert_eq!(
        snapshot["metrics"]["tool_calls"]["send_message"]["success"],
        1
    );
}
