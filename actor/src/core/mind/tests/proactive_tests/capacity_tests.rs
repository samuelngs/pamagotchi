use super::*;

#[tokio::test]
async fn at_capacity_defers_proactive_intent_instead_of_dropping() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    fill_capacity_with_running_responses(&mut mind);
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
    msg.person = Some(person);
    msg.timestamp = recent_timestamp();
    append_inbound(mind.store.as_ref(), &msg).await;
    allow_proactive(&mind, msg.person.as_ref().unwrap());

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Check in".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: None,
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    match decision {
        MindDecision::DeferIntent(intent, delay_secs) => {
            assert_eq!(delay_secs, 60);
            assert_eq!(intent.defer_count, 1);
        }
        _ => panic!("expected deferred intent at capacity"),
    }
}
