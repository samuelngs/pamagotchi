use super::*;

#[tokio::test]
async fn defer_verdict_reemits_message_with_bounded_count() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );

    let decision = mind
        .build_decision(
            MindVerdict::Defer { delay_secs: 999 },
            &WakeEvent::Message(msg),
        )
        .await;

    match decision {
        MindDecision::DeferMessage(deferred, delay_secs) => {
            assert_eq!(delay_secs, 300);
            assert_eq!(deferred.metadata["mind_defer_count"], 1);
        }
        _ => panic!("expected deferred message"),
    }
}
#[tokio::test]
async fn defer_verdict_reemits_intent_with_chosen_human_approval_and_bounded_count() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);
    let intent = FiredIntent {
        id: "intent-chosen-human-approved".into(),
        task: "Follow up after the deploy".into(),
        conversation: Some(ConversationId("relay:local".into())),
        person: Some(PersonId("person-sam".into())),
        scheduled_at: None,
        chosen_human_approved: true,
        defer_count: 1,
    };

    let decision = mind
        .build_decision(
            MindVerdict::Defer { delay_secs: 999 },
            &WakeEvent::IntentFired(intent),
        )
        .await;

    match decision {
        MindDecision::DeferIntent(deferred, delay_secs) => {
            assert_eq!(delay_secs, 300);
            assert_eq!(deferred.id, "intent-chosen-human-approved");
            assert!(deferred.chosen_human_approved);
            assert_eq!(deferred.defer_count, 2);
        }
        _ => panic!("expected deferred intent"),
    }
}
#[tokio::test]
async fn defer_verdict_reemits_consolidation_due() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);

    let decision = mind
        .build_decision(
            MindVerdict::Defer { delay_secs: 999 },
            &WakeEvent::ConsolidationDue,
        )
        .await;

    match decision {
        MindDecision::DeferConsolidation(delay_secs) => assert_eq!(delay_secs, 300),
        _ => panic!("expected deferred consolidation"),
    }
}
#[tokio::test]
async fn at_capacity_defers_new_message_instead_of_dropping() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    fill_capacity_with_running_responses(&mut mind);
    let msg = inbound(
        "relay",
        "incoming",
        "Sam",
        "incoming",
        "relay:incoming",
        None,
        "incoming-msg-1",
    );

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::Message(msg),
        )
        .await;

    match decision {
        MindDecision::DeferMessage(deferred, delay_secs) => {
            assert_eq!(delay_secs, 15);
            assert_eq!(deferred.metadata["mind_defer_count"], 1);
        }
        _ => panic!("expected deferred message at capacity"),
    }
}
#[tokio::test]
async fn deferred_message_timer_keeps_pending_event_when_actor_channel_is_closed() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store.clone());
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-deferred-closed-channel",
    );

    mind.execute_decision(MindDecision::DeferMessage(msg, 0))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let pending = store.pending_events_by_kind("message", 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, "pending");
    assert_eq!(pending[0].attempts, 0);
}
