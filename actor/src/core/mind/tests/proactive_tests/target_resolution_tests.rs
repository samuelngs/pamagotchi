use super::*;

#[tokio::test]
async fn proactive_intent_without_conversation_uses_last_person_conversation() {
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

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Follow up".into(),
                conversation: None,
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    match decision {
        MindDecision::Spawn(action) => {
            assert!(matches!(action.kind, ActionKind::Outreach));
            assert_eq!(
                action.conversation,
                Some(ConversationId("relay:local".into()))
            );
            assert_eq!(action.source_intent.as_deref(), Some("intent-1"));
        }
        _ => panic!("expected outreach spawn with inferred conversation"),
    }
}
#[tokio::test]
async fn proactive_intent_without_conversation_uses_channel_preference_when_available() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let person = PersonId("person-sam".into());
    let now = recent_timestamp();
    let mut preferred = inbound(
        "relay",
        "preferred",
        "Sam",
        "preferred",
        "relay:preferred",
        None,
        "msg-preferred",
    );
    preferred.person = Some(person.clone());
    preferred.timestamp = now - 120;
    let mut recent = inbound(
        "relay",
        "recent",
        "Sam",
        "recent",
        "relay:recent",
        None,
        "msg-recent",
    );
    recent.person = Some(person.clone());
    recent.timestamp = now - 10;
    append_inbound(store.as_ref(), &preferred).await;
    append_inbound(store.as_ref(), &recent).await;
    {
        let mut actor = mind.state.shared.actor.write().unwrap();
        let rel = actor.bonds.entry(person.clone()).or_default();
        rel.proactive_consent = ProactiveConsent::Allowed;
        rel.channel_preference = Some("Use relay:preferred for proactive check-ins".into());
    }

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Follow up".into(),
                conversation: None,
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    match decision {
        MindDecision::Spawn(action) => {
            assert!(matches!(action.kind, ActionKind::Outreach));
            assert_eq!(
                action.conversation,
                Some(ConversationId("relay:preferred".into()))
            );
        }
        _ => panic!("expected outreach spawn with preferred conversation"),
    }
}
#[tokio::test]
async fn proactive_intent_with_unknown_conversation_person_drops() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Follow up".into(),
                conversation: Some(ConversationId("relay:unknown".into())),
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
async fn proactive_intent_for_stale_conversation_drops() {
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
    msg.person = Some(person);
    msg.timestamp = recent_timestamp() - 31 * 24 * 60 * 60;
    append_inbound(store.as_ref(), &msg).await;
    allow_proactive(&mind, msg.person.as_ref().unwrap());

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
#[tokio::test]
async fn proactive_intent_without_any_target_drops() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store);

    let decision = mind
        .build_decision(
            MindVerdict::Respond {
                style_directive: None,
            },
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-1".into(),
                task: "Follow up".into(),
                conversation: None,
                person: None,
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    assert!(matches!(decision, MindDecision::Drop));
}
