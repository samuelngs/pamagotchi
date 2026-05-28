use super::*;

#[tokio::test]
async fn scheduler_leaves_due_intent_active_when_actor_channel_is_closed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    store
        .create_intent(&IntentRecord {
            id: "intent-message".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check in".into(),
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
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    assert!(!drain_due_intents(&tx, store_dyn, 1000, 10).await);

    let due = store.due_intents(1001, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "intent-message");
    assert_eq!(due[0].status, "active");
}
#[tokio::test]
async fn claimed_due_intent_is_not_emitted_twice() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    store
        .create_intent(&IntentRecord {
            id: "intent-message".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check in".into(),
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
            chosen_person_approved: false,
        })
        .await
        .unwrap();
    assert!(
        store
            .mark_intent_fired("intent-message", 1000)
            .await
            .unwrap()
    );

    let intent = store.get_intent("intent-message").await.unwrap().unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    assert!(claim_and_send_due_intent(&tx, store_dyn.as_ref(), intent, 1001).await);

    assert!(rx.try_recv().is_err());
    assert!(store.due_intents(1001, 10).await.unwrap().is_empty());
}
#[tokio::test]
async fn scheduler_drains_due_intent_after_actor_handoff() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    store
        .create_intent(&IntentRecord {
            id: "intent-message".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Check in".into(),
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
            chosen_person_approved: true,
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel(1);
    assert!(drain_due_intents(&tx, store_dyn, 1000, 10).await);

    match rx.recv().await.unwrap() {
        WakeEvent::IntentFired(intent) => {
            assert_eq!(intent.id, "intent-message");
            assert_eq!(intent.task, "Check in");
            assert_eq!(
                intent.conversation,
                Some(ConversationId("relay:local".into()))
            );
            assert_eq!(intent.person, Some(PersonId("person-sam".into())));
            assert_eq!(intent.scheduled_at, Some(900));
            assert!(intent.chosen_person_approved);
        }
        _ => panic!("expected fired intent"),
    }
    assert!(store.due_intents(1001, 10).await.unwrap().is_empty());
    assert_eq!(
        store
            .get_intent("intent-message")
            .await
            .unwrap()
            .unwrap()
            .status,
        "fired"
    );
}
#[tokio::test]
async fn scheduler_preserves_chosen_person_approval_on_deferred_intent_events() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let intent = FiredIntent {
        id: "intent-chosen-person-approved".into(),
        task: "Check in".into(),
        conversation: Some(ConversationId("relay:local".into())),
        person: Some(PersonId("person-sam".into())),
        scheduled_at: Some(900),
        chosen_person_approved: true,
        defer_count: 1,
    };
    store
        .enqueue_event(&EventInboxRecord {
            id: "event-intent".into(),
            kind: "intent_fired".into(),
            payload: serde_json::to_value(&intent).unwrap(),
            status: "pending".into(),
            due_at: 1000,
            attempts: 0,
            dedupe_key: None,
            created_at: 900,
            updated_at: 900,
            fired_at: None,
            last_error: None,
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel(1);
    assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

    match rx.recv().await.unwrap() {
        WakeEvent::IntentFired(intent) => {
            assert_eq!(intent.id, "intent-chosen-person-approved");
            assert_eq!(intent.scheduled_at, Some(900));
            assert!(intent.chosen_person_approved);
            assert_eq!(intent.defer_count, 1);
        }
        _ => panic!("expected deferred intent event"),
    }
}
