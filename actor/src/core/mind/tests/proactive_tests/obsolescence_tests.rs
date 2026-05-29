use super::*;

#[tokio::test]
async fn proactive_intent_drops_when_last_visible_message_is_assistant() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let conversation = ConversationId("relay:local".into());
    let person = PersonId("person-sam".into());
    let now = recent_timestamp();
    allow_proactive(&mind, &person);
    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now - 10,
                role: MessageRole::User,
                content: "ping me later".into(),
                identity: None,
                profile: None,
                person: Some(person.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now - 5,
                role: MessageRole::Assistant,
                content: "will do".into(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Check in".into(),
                conversation: Some(conversation),
                person: None,
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
#[tokio::test]
async fn proactive_intent_drops_when_target_replied_after_scheduling() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let conversation = ConversationId("relay:local".into());
    let person = PersonId("person-sam".into());
    let now = recent_timestamp();
    allow_proactive(&mind, &person);

    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now - 300,
                role: MessageRole::User,
                content: "can you check in later?".into(),
                identity: None,
                profile: None,
                person: Some(person.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-before-intent".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-obsolete-followup".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check in with Sam".into(),
            person: Some(person.clone()),
            profile: None,
            conversation: Some(conversation.clone()),
            fire_at: Some(now - 10),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: Some("review-action".into()),
            source_memory: None,
            created_at: now - 240,
            updated_at: now - 240,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now - 60,
                role: MessageRole::User,
                content: "actually, I sorted it out".into(),
                identity: None,
                profile: None,
                person: Some(person.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-after-intent".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .mark_intent_fired("intent-obsolete-followup", now)
            .await
            .unwrap()
    );

    let event = WakeEvent::IntentFired(FiredIntent {
        id: "intent-obsolete-followup".into(),
        task: "Check in with Sam".into(),
        conversation: Some(conversation),
        person: Some(person),
        scheduled_at: Some(now - 240),
        chosen_human_approved: false,
        defer_count: 0,
    });
    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &event,
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
    mind.retire_dropped_fired_intent(&event, &decision).await;
    let stored = store
        .get_intent("intent-obsolete-followup")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "completed");
}
#[tokio::test]
async fn proactive_intent_uses_latest_schedule_time_for_reply_obsolescence() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let conversation = ConversationId("relay:local".into());
    let person = PersonId("person-sam".into());
    let now = recent_timestamp();
    allow_proactive(&mind, &person);

    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now - 120,
                role: MessageRole::User,
                content: "I replied before you rescheduled the check-in.".into(),
                identity: None,
                profile: None,
                person: Some(person.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-before-reschedule".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-rescheduled-followup".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check in after Sam's reply".into(),
            person: Some(person.clone()),
            profile: None,
            conversation: Some(conversation.clone()),
            fire_at: Some(now - 10),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: Some("review-action".into()),
            source_memory: None,
            created_at: now - 600,
            updated_at: now - 30,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    assert!(
        store
            .mark_intent_fired("intent-rescheduled-followup", now)
            .await
            .unwrap()
    );

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-rescheduled-followup".into(),
                task: "Check in after Sam's reply".into(),
                conversation: Some(conversation),
                person: Some(person),
                scheduled_at: Some(now - 30),
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(
        matches!(decision, MindDecision::Spawn(action) if matches!(action.kind, ActionKind::Outreach))
    );
}
#[tokio::test]
async fn proactive_intent_drops_when_prior_proactive_outreach_is_unanswered() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let person = PersonId("person-sam".into());
    let mut msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "msg-1",
    );
    msg.person = Some(person.clone());
    msg.timestamp = recent_timestamp();
    append_inbound(store.as_ref(), &msg).await;
    allow_proactive(&mind, &person);
    set_unanswered_proactive_outreach(&mind, &person);

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Follow up".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: None,
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
