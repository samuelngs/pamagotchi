use super::*;

#[tokio::test]
async fn proactive_intent_with_allowed_consent_spawns_outreach() {
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
                conversation: Some(ConversationId("relay:local".into())),
                person: None,
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
        _ => panic!("expected proactive outreach spawn with consent"),
    }
}
#[tokio::test]
async fn proactive_intent_defers_when_gateway_is_disconnected() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind_with_gateway_state(store.clone(), GatewayConnectionState::Disconnected);
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
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(person),
                scheduled_at: None,
                chosen_human_approved: false,
                defer_count: 0,
            }),
        )
        .await;

    match decision {
        MindDecision::DeferIntent(intent, delay_secs) => {
            assert_eq!(intent.defer_count, 1);
            assert_eq!(delay_secs, 5 * 60);
        }
        _ => panic!("expected proactive outreach to defer while gateway is disconnected"),
    }
}
#[tokio::test]
async fn proactive_intent_during_quiet_hours_defers() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let now = chrono::Utc::now();
    {
        use chrono::Timelike;
        let start_hour = now.hour() as u8;
        let end_hour = ((now.hour() + 1) % 24) as u8;
        mind.state
            .shared
            .config
            .write()
            .unwrap()
            .proactivity
            .quiet_hours_utc = Some(QuietHoursUtc {
            start_hour,
            end_hour,
        });
    }
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

    match decision {
        MindDecision::DeferIntent(intent, delay_secs) => {
            assert_eq!(intent.defer_count, 1);
            assert!((60..=3600).contains(&delay_secs));
        }
        _ => panic!("expected quiet-hours intent deferral"),
    }
}
