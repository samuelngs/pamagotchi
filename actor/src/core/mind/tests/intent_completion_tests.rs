use super::*;

#[tokio::test]
async fn successful_response_retires_simple_next_message_triggered_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store.clone());
    let person = PersonId("person-sam".into());
    let profile = ProfileId("profile-sam".into());
    let mut msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    msg.person = Some(person.clone());
    msg.profile = Some(profile.clone());
    let now = recent_timestamp();

    store
        .create_intent(&IntentRecord {
            id: "intent-next-message".into(),
            kind: "triggered".into(),
            status: "active".into(),
            task: "Ask how the deployment went".into(),
            person: Some(person.clone()),
            profile: Some(profile.clone()),
            conversation: Some(msg.conversation.clone()),
            fire_at: None,
            condition: Some("next time Sam messages".into()),
            recurrence: None,
            priority: 80,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-specific-condition".into(),
            kind: "triggered".into(),
            status: "active".into(),
            task: "Ask about the deployment only if Sam brings it up".into(),
            person: Some(person.clone()),
            profile: Some(profile),
            conversation: Some(msg.conversation.clone()),
            fire_at: None,
            condition: Some("when Sam mentions deployment".into()),
            recurrence: None,
            priority: 70,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let action = Action::respond(
        vec![msg.clone()],
        msg.conversation.clone(),
        Authority::Default,
        None,
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: true,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        },
    );

    mind.retire_handled_triggered_intents(&id).await;

    let retired = store
        .get_intent("intent-next-message")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retired.status, "completed");
    assert!(retired.last_fired_at.is_none());

    let still_active = store
        .get_intent("intent-specific-condition")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still_active.status, "active");
    assert!(still_active.last_fired_at.is_none());
}
#[tokio::test]
async fn successful_response_retires_matched_content_triggered_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store.clone());
    let person = PersonId("person-sam".into());
    let profile = ProfileId("profile-sam".into());
    let mut msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    msg.content = "Deployment finished, but the rollback notes are messy.".into();
    msg.person = Some(person.clone());
    msg.profile = Some(profile.clone());
    let now = recent_timestamp();

    store
        .create_intent(&IntentRecord {
            id: "intent-deployment-condition".into(),
            kind: "triggered".into(),
            status: "active".into(),
            task: "Ask about the deployment only if Sam brings it up".into(),
            person: Some(person.clone()),
            profile: Some(profile.clone()),
            conversation: Some(msg.conversation.clone()),
            fire_at: None,
            condition: Some("when Sam mentions deployment".into()),
            recurrence: None,
            priority: 70,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();
    store
        .create_intent(&IntentRecord {
            id: "intent-budget-condition".into(),
            kind: "triggered".into(),
            status: "active".into(),
            task: "Ask about budget only if Sam brings it up".into(),
            person: Some(person.clone()),
            profile: Some(profile),
            conversation: Some(msg.conversation.clone()),
            fire_at: None,
            condition: Some("when Sam asks about budget".into()),
            recurrence: None,
            priority: 60,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let action = Action::respond(
        vec![msg.clone()],
        msg.conversation.clone(),
        Authority::Default,
        None,
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: true,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        },
    );

    mind.retire_handled_triggered_intents(&id).await;

    let matched = store
        .get_intent("intent-deployment-condition")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(matched.status, "completed");
    assert!(matched.last_fired_at.is_none());

    let unmatched = store
        .get_intent("intent-budget-condition")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unmatched.status, "active");
    assert!(unmatched.last_fired_at.is_none());
}
#[tokio::test]
async fn successful_outreach_completes_fired_source_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let conversation = ConversationId("relay:local".into());
    let now = recent_timestamp();
    store
        .create_intent(&IntentRecord {
            id: "intent-outreach".into(),
            kind: "proactive".into(),
            status: "fired".into(),
            task: "Check in about the deployment".into(),
            person: None,
            profile: None,
            conversation: Some(conversation.clone()),
            fire_at: Some(now - 60),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now - 120,
            updated_at: now - 60,
            last_fired_at: Some(now - 60),
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let mut mind = mind;
    let action = Action::outreach_with_source_intent(
        "Check in about the deployment".into(),
        Some(conversation),
        Authority::Default,
        Some("intent-outreach".into()),
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: true,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        },
    );

    mind.complete_successful_outreach_source_intent(&id).await;

    let completed = store.get_intent("intent-outreach").await.unwrap().unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.last_fired_at, Some(now - 60));
}
#[tokio::test]
async fn successful_recurring_outreach_keeps_active_source_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store.clone());
    let conversation = ConversationId("relay:local".into());
    let now = recent_timestamp();
    store
        .create_intent(&IntentRecord {
            id: "intent-recurring-outreach".into(),
            kind: "proactive".into(),
            status: "active".into(),
            task: "Weekly check-in".into(),
            person: None,
            profile: None,
            conversation: Some(conversation.clone()),
            fire_at: Some(now + 60 * 60),
            condition: None,
            recurrence: Some("weekly".into()),
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: now - 120,
            updated_at: now - 60,
            last_fired_at: Some(now - 60),
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let action = Action::outreach_with_source_intent(
        "Weekly check-in".into(),
        Some(conversation),
        Authority::Default,
        Some("intent-recurring-outreach".into()),
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: true,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        },
    );

    mind.complete_successful_outreach_source_intent(&id).await;

    let active = store
        .get_intent("intent-recurring-outreach")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.status, "active");
    assert_eq!(active.recurrence.as_deref(), Some("weekly"));
    assert_eq!(active.last_fired_at, Some(now - 60));
}
