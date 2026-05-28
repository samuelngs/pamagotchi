use super::*;

#[tokio::test]
async fn intents_are_persisted_updated_due_and_fired_once() {
    let store = test_store();
    let intent = IntentRecord {
        id: "intent-1".into(),
        kind: "scheduled".into(),
        status: "active".into(),
        task: "Ask how the deployment went".into(),
        person: Some(PersonId("sam".into())),
        profile: Some(ProfileId("profile-sam".into())),
        conversation: Some(ConversationId("relay:local".into())),
        fire_at: Some(1000),
        condition: None,
        recurrence: None,
        priority: 80,
        dedupe_key: Some("followup:deploy".into()),
        source_action: Some("action-1".into()),
        source_memory: Some(MemoryId("memory-commitment-1".into())),
        created_at: 900,
        updated_at: 900,
        last_fired_at: None,
        chosen_person_approved: true,
    };

    store.create_intent(&intent).await.unwrap();
    let stored = store.get_intent("intent-1").await.unwrap().unwrap();
    assert_eq!(stored.task, "Ask how the deployment went");
    assert!(stored.chosen_person_approved);
    assert_eq!(
        stored.source_memory.as_ref().map(|id| id.0.as_str()),
        Some("memory-commitment-1")
    );

    store
        .update_intent(
            "intent-1",
            &IntentUpdateRecord {
                task: Some("Ask whether the deployment recovered".into()),
                priority: Some(90),
                source_memory: Some(MemoryId("memory-commitment-2".into())),
                chosen_person_approved: Some(false),
                updated_at: 950,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let due = store.due_intents(1000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].priority, 90);
    assert_eq!(due[0].task, "Ask whether the deployment recovered");
    assert!(!due[0].chosen_person_approved);
    assert_eq!(
        due[0].source_memory.as_ref().map(|id| id.0.as_str()),
        Some("memory-commitment-2")
    );

    assert!(store.mark_intent_fired("intent-1", 1001).await.unwrap());
    assert!(!store.mark_intent_fired("intent-1", 1002).await.unwrap());
    assert!(store.due_intents(2000, 10).await.unwrap().is_empty());

    let fired = store.get_intent("intent-1").await.unwrap().unwrap();
    assert_eq!(fired.status, "fired");
    assert_eq!(fired.last_fired_at, Some(1001));
}
#[tokio::test]
async fn intents_can_be_marked_completed_once() {
    let store = test_store();
    store
        .create_intent(&IntentRecord {
            id: "intent-complete".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Close resolved loop".into(),
            person: Some(PersonId("sam".into())),
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
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    assert!(
        store
            .complete_intent("intent-complete", 1100)
            .await
            .unwrap()
    );
    assert!(
        !store
            .complete_intent("intent-complete", 1200)
            .await
            .unwrap()
    );
    assert!(store.due_intents(2000, 10).await.unwrap().is_empty());

    let completed = store.get_intent("intent-complete").await.unwrap().unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.updated_at, 1100);
}
#[tokio::test]
async fn cancelled_intents_are_not_marked_completed() {
    let store = test_store();
    store
        .create_intent(&IntentRecord {
            id: "intent-cancelled".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Cancelled loop".into(),
            person: Some(PersonId("sam".into())),
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
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    assert!(store.cancel_intent("intent-cancelled", 1000).await.unwrap());
    assert!(
        !store
            .complete_intent("intent-cancelled", 1100)
            .await
            .unwrap()
    );

    let cancelled = store.get_intent("intent-cancelled").await.unwrap().unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(cancelled.updated_at, 1000);
}
#[tokio::test]
async fn recurring_intents_reschedule_when_fired() {
    let store = test_store();
    store
        .create_intent(&IntentRecord {
            id: "intent-recurring".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check weekly project status".into(),
            person: Some(PersonId("sam".into())),
            profile: None,
            conversation: Some(ConversationId("relay:local".into())),
            fire_at: Some(1000),
            condition: None,
            recurrence: Some("every 2 hours".into()),
            priority: 80,
            dedupe_key: Some("followup:recurring-project-status".into()),
            source_action: Some("action-1".into()),
            source_memory: None,
            created_at: 900,
            updated_at: 900,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    assert_eq!(
        store.due_intents(1000, 10).await.unwrap()[0].id,
        "intent-recurring"
    );
    assert!(
        store
            .mark_intent_fired("intent-recurring", 1001)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_intent_fired("intent-recurring", 1002)
            .await
            .unwrap()
    );

    let rescheduled = store.get_intent("intent-recurring").await.unwrap().unwrap();
    assert_eq!(rescheduled.status, "active");
    assert_eq!(rescheduled.last_fired_at, Some(1001));
    assert_eq!(rescheduled.fire_at, Some(8200));
    assert!(store.due_intents(8199, 10).await.unwrap().is_empty());
    assert_eq!(
        store.due_intents(8200, 10).await.unwrap()[0].id,
        "intent-recurring"
    );

    assert!(
        store
            .mark_intent_fired("intent-recurring", 20_000)
            .await
            .unwrap()
    );
    let rescheduled = store.get_intent("intent-recurring").await.unwrap().unwrap();
    assert_eq!(rescheduled.status, "active");
    assert_eq!(rescheduled.last_fired_at, Some(20_000));
    assert_eq!(rescheduled.fire_at, Some(22_600));
}
