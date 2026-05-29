use super::*;
use crate::core::{ActorLifecycleEvent, MindLifecycleDecision, MindLifecycleVerdict};
use std::time::Duration;

fn test_mind_with_lifecycle(
    store: Arc<dyn Store>,
) -> (
    Mind,
    mpsc::Sender<WakeEvent>,
    mpsc::UnboundedReceiver<ActorLifecycleEvent>,
) {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (state_tx, _state_rx) = mpsc::channel(4);
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let shared = Arc::new(SharedState {
        actor: RwLock::new(ActorState::new(Default::default())),
        config: RwLock::new(GrowthConfig::default()),
    });
    let state = StateHandle::new(shared, state_tx);
    let gateway = Arc::new(GatewayRouter::new());
    gateway.register(Arc::new(StateAdapter {
        state: GatewayConnectionState::Connected,
    }));

    (
        Mind::new(
            event_rx,
            event_tx.clone(),
            state,
            store,
            None,
            test_router(),
            gateway,
            5,
            5,
            1,
            1,
            Arc::new(ActorMetrics::default()),
            Some(lifecycle_tx),
        ),
        event_tx,
        lifecycle_rx,
    )
}

async fn collect_until_stopped(
    lifecycle_rx: &mut mpsc::UnboundedReceiver<ActorLifecycleEvent>,
) -> Vec<ActorLifecycleEvent> {
    let mut events = vec![];
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), lifecycle_rx.recv())
            .await
            .expect("lifecycle event timed out")
            .expect("lifecycle channel closed before mind stopped");
        let stopped = matches!(event, ActorLifecycleEvent::MindStopped);
        events.push(event);
        if stopped {
            return events;
        }
    }
}

fn event_index(
    events: &[ActorLifecycleEvent],
    predicate: impl Fn(&ActorLifecycleEvent) -> bool,
) -> usize {
    events
        .iter()
        .position(predicate)
        .expect("expected lifecycle event")
}

#[tokio::test]
async fn run_emits_mind_lifecycle_and_decision_summaries() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let (mind, event_tx, mut lifecycle_rx) = test_mind_with_lifecycle(store);
    let join = tokio::spawn(async move {
        mind.run().await;
    });

    let intent = FiredIntent {
        id: "intent-lifecycle".into(),
        task: "Check in later".into(),
        conversation: None,
        person: None,
        scheduled_at: None,
        chosen_human_approved: false,
        defer_count: 0,
    };
    event_tx
        .send(WakeEvent::IntentFired(intent))
        .await
        .expect("send intent");
    event_tx
        .send(WakeEvent::Shutdown)
        .await
        .expect("send shutdown");
    drop(event_tx);

    let events = collect_until_stopped(&mut lifecycle_rx).await;
    join.await.expect("mind task panicked");

    let started = event_index(&events, |event| {
        matches!(event, ActorLifecycleEvent::MindStarted)
    });
    let evaluated = event_index(&events, |event| {
        matches!(event, ActorLifecycleEvent::MindEvaluated(_))
    });
    let decision_built = event_index(&events, |event| {
        matches!(event, ActorLifecycleEvent::MindDecisionBuilt(_))
    });
    let stopped = event_index(&events, |event| {
        matches!(event, ActorLifecycleEvent::MindStopped)
    });
    assert!(started < evaluated);
    assert!(evaluated < decision_built);
    assert!(decision_built < stopped);

    let ActorLifecycleEvent::MindEvaluated(evaluation) = &events[evaluated] else {
        unreachable!();
    };
    assert_eq!(evaluation.wake.kind, "intent_fired");
    assert_eq!(evaluation.wake.conversation, None);
    assert!(evaluation.wake.source_message_keys.is_empty());
    assert_eq!(evaluation.wake.intent_id, Some("intent-lifecycle".into()));
    assert!(matches!(
        evaluation.verdict,
        MindLifecycleVerdict::Respond {
            has_style_directive: false
        }
    ));

    let ActorLifecycleEvent::MindDecisionBuilt(decision) = &events[decision_built] else {
        unreachable!();
    };
    assert_eq!(decision.wake.kind, "intent_fired");
    assert!(matches!(decision.decision, MindLifecycleDecision::Drop));
}
