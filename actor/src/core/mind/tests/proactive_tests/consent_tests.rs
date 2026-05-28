use super::*;

#[tokio::test]
async fn proactive_intent_with_unknown_consent_drops() {
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
    set_proactive_consent(&mind, &person, ProactiveConsent::Unknown);

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
                chosen_person_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
#[tokio::test]
async fn proactive_intent_with_denied_consent_drops() {
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
    set_proactive_consent(&mind, &person, ProactiveConsent::Denied);

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
                chosen_person_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
