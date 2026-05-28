use super::*;

#[tokio::test]
async fn message_revision_events_update_stored_conversation_history() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mind = test_mind(store.clone());
    let conv = ConversationId("relay:local".into());
    store
        .append_message(
            &conv,
            Some("relay"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "before edit".into(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    mind.apply_message_edit(&conv, "relay", "msg-1", "after edit", 1100)
        .await;
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "after edit");
    assert_eq!(messages[0].metadata["edited_at"], 1100);

    mind.apply_message_delete(&conv, "relay", "msg-1", 1200)
        .await;
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "[message deleted]");
    assert_eq!(messages[0].metadata["deleted_at"], 1200);
}
#[tokio::test]
async fn mind_metrics_track_decisions_and_action_queue() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );

    mind.execute_decision(MindDecision::Drop).await;
    mind.execute_decision(MindDecision::Inject(
        ActionId("missing-action".into()),
        msg.clone(),
    ))
    .await;
    mind.execute_decision(MindDecision::DeferMessage(msg, 300))
        .await;
    let queued_id = mind.schedule_action(Action::ruminate());
    let snapshot = mind.metrics.snapshot();
    assert_eq!(snapshot.events_dropped, 1);
    assert_eq!(snapshot.events_deferred, 1);
    assert_eq!(snapshot.injection_failures, 1);
    assert_eq!(snapshot.actions_spawned, 1);
    assert_eq!(snapshot.action_queue_length, 1);
    assert_eq!(snapshot.running_actions, 0);

    mind.registry.launch(&queued_id).expect("action launches");
    mind.refresh_registry_metrics();
    let snapshot = mind.metrics.snapshot();
    assert_eq!(snapshot.action_queue_length, 0);
    assert_eq!(snapshot.running_actions, 1);
}
#[tokio::test]
async fn failed_running_action_injection_requeues_message_and_skips_dead_target() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (mut mind, mut external_rx) =
        test_mind_with_gateway_state_and_event_receiver(store, GatewayConnectionState::Connected);
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-injected",
    );
    let id = mind.schedule_action(Action::respond(
        vec![msg.clone()],
        msg.conversation.clone(),
        Authority::Default,
        None,
    ));
    let launch = mind.registry.launch(&id).expect("action launches");
    drop(launch);

    mind.execute_decision(MindDecision::Inject(id.clone(), msg.clone()))
        .await;

    let requeued = match external_rx.recv().await.expect("message requeued") {
        WakeEvent::Message(msg) => msg,
        _ => panic!("expected requeued message"),
    };
    assert!(message_skips_injection_target(&requeued, &id));

    let decision = mind
        .respond_to(&WakeEvent::Message(requeued.clone()), None)
        .await;
    match decision {
        MindDecision::Spawn(action) => {
            assert!(matches!(action.kind, ActionKind::Respond));
            assert_eq!(action.source_messages[0].message_id, requeued.message_id);
        }
        _ => panic!("expected fresh respond action instead of retrying dead injection"),
    }
}
#[tokio::test]
async fn cancelling_running_action_leaves_composing_release_to_session_guard() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    let action = Action::respond(
        vec![msg.clone()],
        msg.conversation.clone(),
        Authority::Default,
        None,
    );
    let running_id = mind.registry.schedule(action);
    let _launch = mind.registry.launch(&running_id).expect("action launches");
    mind.gateway.acquire_composing("relay", "local").await;
    assert_eq!(mind.gateway.composing_count("relay", "local").await, 1);

    let replacement = Action::ruminate();
    mind.execute_decision(MindDecision::CancelAndSpawn(vec![running_id], replacement))
        .await;

    assert_eq!(mind.gateway.composing_count("relay", "local").await, 1);
    mind.gateway.release_composing("relay", "local").await;
    assert_eq!(mind.gateway.composing_count("relay", "local").await, 0);
}
