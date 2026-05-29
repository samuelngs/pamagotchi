use super::*;

#[tokio::test]
async fn restricted_intent_does_not_spawn_outreach() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);
    let person = PersonId("restricted-person".into());
    mind.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&person, Some(Authority::Restricted));

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Check in".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
#[tokio::test]
async fn blocked_intent_without_chosen_human_approval_drops() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);
    let person = PersonId("blocked-person".into());
    mind.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&person, Some(Authority::Blocked));

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Check in".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
#[tokio::test]
async fn chosen_human_approved_restricted_intent_can_spawn_outreach() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let person = PersonId("restricted-person".into());
    mind.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&person, Some(Authority::Restricted));
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

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Check in".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: true,
                defer_count: 0,
            }),
        )
        .await;

    match decision {
        MindDecision::Spawn(action) => {
            assert!(matches!(action.kind, ActionKind::Outreach));
            assert_eq!(action.authority, Authority::Restricted);
        }
        _ => panic!("expected chosen-human-approved restricted outreach to spawn"),
    }
}
#[tokio::test]
async fn chosen_human_approved_blocked_intent_can_spawn_outreach() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let person = PersonId("blocked-person".into());
    mind.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&person, Some(Authority::Blocked));
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

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Check in".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: true,
                defer_count: 0,
            }),
        )
        .await;

    match decision {
        MindDecision::Spawn(action) => {
            assert!(matches!(action.kind, ActionKind::Outreach));
            assert_eq!(action.authority, Authority::Blocked);
        }
        _ => panic!("expected chosen-human-approved blocked outreach to spawn"),
    }
}
#[tokio::test]
async fn chosen_human_approved_intent_still_respects_denied_proactive_consent() {
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
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: true,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
