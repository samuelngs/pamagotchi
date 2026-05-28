use super::*;

fn successful_outcome() -> Outcome {
    Outcome {
        responded: true,
        attempted_send: true,
        attempts: 1,
        ..Outcome::default()
    }
}

#[test]
fn gc_retains_recent_completed_actions_for_observability() {
    let mut registry = ActionRegistry::new(1);
    let id = registry.schedule(Action::ruminate());

    registry.complete(&id, successful_outcome());
    registry.gc();

    assert!(registry.get(&id).is_some());
    assert_eq!(registry.recent_completed().len(), 1);
    assert!(matches!(
        registry.get(&id).unwrap().phase,
        Phase::Done { .. }
    ));
}

#[test]
fn gc_prunes_completed_actions_after_recent_window() {
    let mut registry = ActionRegistry::new(1);
    let mut ids = Vec::new();

    for _ in 0..(MAX_RECENT_COMPLETED_ACTIONS + 2) {
        let id = registry.schedule(Action::ruminate());
        registry.complete(&id, successful_outcome());
        ids.push(id);
    }
    registry.gc();

    assert!(registry.get(&ids[0]).is_none());
    assert!(registry.get(&ids[1]).is_none());
    assert!(registry.get(ids.last().unwrap()).is_some());
    assert_eq!(
        registry.recent_completed().len(),
        MAX_RECENT_COMPLETED_ACTIONS
    );
}

#[test]
fn completed_actions_do_not_count_toward_capacity() {
    let mut registry = ActionRegistry::new(1);
    let id = registry.schedule(Action::ruminate());
    registry.complete(&id, successful_outcome());

    assert!(!registry.at_capacity());
    assert!(registry.running().is_empty());
    assert_eq!(registry.recent_completed().len(), 1);
}

#[tokio::test]
async fn cancel_requests_cooperative_stop_without_aborting_task() {
    let mut registry = ActionRegistry::new(1);
    let id = registry.schedule(Action::ruminate());
    let launch = registry.launch(&id).expect("action launched");
    let progress = launch.progress.clone();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        release_rx.await.ok();
        finished_tx.send(()).ok();
    });
    registry.set_handle(&id, handle);

    assert!(registry.cancel(&id));
    assert!(progress.read().unwrap().is_cancelled());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), finished_rx)
            .await
            .is_err(),
        "cancel should not hard-abort the task"
    );

    release_tx.send(()).ok();
}

#[test]
fn failed_delivery_outcome_does_not_requeue_source_message() {
    let mut registry = ActionRegistry::new(1);
    let source = protocol::InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "relay".into(),
        sender_external_id: "local".into(),
        sender_display_name: None,
        reply_external_id: "local".into(),
        conversation: ConversationId("relay:local".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
        content: "hello".into(),
        attachments: vec![],
        timestamp: 1,
        metadata: serde_json::Value::Null,
    };
    let id = registry.schedule(Action::respond(
        vec![source],
        ConversationId("relay:local".into()),
        crate::state::Authority::Default,
        None,
    ));

    registry.complete(
        &id,
        Outcome {
            responded: false,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        },
    );

    assert!(registry.follow_ups(&id).is_empty());
}
