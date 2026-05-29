use super::*;

#[tokio::test]
async fn dropped_one_shot_fired_intent_is_completed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    store
        .create_intent(&IntentRecord {
            id: "intent-one-shot".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Follow up once".into(),
            person: Some(PersonId("person-sam".into())),
            profile: None,
            conversation: Some(ConversationId("relay:local".into())),
            fire_at: Some(1000),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 900,
            updated_at: 900,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    assert!(
        store
            .mark_intent_fired("intent-one-shot", 1000)
            .await
            .unwrap()
    );

    mind.retire_dropped_fired_intent(
        &WakeEvent::IntentFired(FiredIntent {
            id: "intent-one-shot".into(),
            task: "Follow up once".into(),
            conversation: Some(ConversationId("relay:local".into())),
            person: Some(PersonId("person-sam".into())),
            scheduled_at: None,
            chosen_human_approved: false,
            defer_count: 0,
        }),
        &MindDecision::Drop,
    )
    .await;

    let retired = store.get_intent("intent-one-shot").await.unwrap().unwrap();
    assert_eq!(retired.status, "completed");
}
#[tokio::test]
async fn dropped_recurring_fired_intent_stays_active_for_next_fire() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    store
        .create_intent(&IntentRecord {
            id: "intent-recurring".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Follow up weekly".into(),
            person: Some(PersonId("person-sam".into())),
            profile: None,
            conversation: Some(ConversationId("relay:local".into())),
            fire_at: Some(1000),
            condition: None,
            recurrence: Some("every 2 hours".into()),
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 900,
            updated_at: 900,
            last_fired_at: None,
            chosen_human_approved: false,
        })
        .await
        .unwrap();
    assert!(
        store
            .mark_intent_fired("intent-recurring", 1000)
            .await
            .unwrap()
    );

    mind.retire_dropped_fired_intent(
        &WakeEvent::IntentFired(FiredIntent {
            id: "intent-recurring".into(),
            task: "Follow up weekly".into(),
            conversation: Some(ConversationId("relay:local".into())),
            person: Some(PersonId("person-sam".into())),
            scheduled_at: None,
            chosen_human_approved: false,
            defer_count: 0,
        }),
        &MindDecision::Drop,
    )
    .await;

    let recurring = store.get_intent("intent-recurring").await.unwrap().unwrap();
    assert_eq!(recurring.status, "active");
    assert_eq!(recurring.last_fired_at, Some(1000));
    assert_eq!(recurring.fire_at, Some(8200));
}
