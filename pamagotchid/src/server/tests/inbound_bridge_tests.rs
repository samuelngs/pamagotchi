use super::*;

#[tokio::test]
async fn inbound_bridge_persists_claims_and_forwards_message() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (event_tx, mut event_rx) = mpsc::channel(2);
    let inbound_tx = inbound_bridge(event_tx, store_dyn);
    let msg = test_inbound("bridge-msg-1");

    inbound_tx.send(msg.clone()).await.unwrap();

    match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap()
    {
        WakeEvent::Message(forwarded) => assert_eq!(forwarded.message_id, msg.message_id),
        _ => panic!("expected forwarded inbound message"),
    }

    assert!(
        !store
            .mark_event_fired(&inbound_event_id(&msg), now_secs())
            .await
            .unwrap()
    );
    assert!(
        store
            .due_events(now_secs() + 1, 10)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn inbound_bridge_suppresses_duplicate_claimed_source_message() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (event_tx, mut event_rx) = mpsc::channel(2);
    let inbound_tx = inbound_bridge(event_tx, store_dyn);
    let msg = test_inbound("bridge-msg-duplicate");

    inbound_tx.send(msg.clone()).await.unwrap();
    inbound_tx.send(msg).await.unwrap();

    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), event_rx.recv())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn inbound_bridge_leaves_pending_event_when_actor_channel_is_closed() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (event_tx, event_rx) = mpsc::channel(1);
    drop(event_rx);
    let inbound_tx = inbound_bridge(event_tx, store_dyn);
    let msg = test_inbound("bridge-msg-pending");
    let event_id = inbound_event_id(&msg);

    inbound_tx.send(msg).await.unwrap();

    let mut due = Vec::new();
    for _ in 0..20 {
        due = store.due_events(now_secs() + 1, 10).await.unwrap();
        if !due.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, event_id);
    assert_eq!(due[0].status, "pending");
}
#[tokio::test]
async fn inbound_bridge_leaves_overflow_pending_and_keeps_accepting_messages() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (event_tx, mut event_rx) = mpsc::channel(1);
    let inbound_tx = inbound_bridge(event_tx, store_dyn);
    let first = test_inbound("bridge-overflow-1");
    let mut second = test_inbound("bridge-overflow-2");
    second.conversation = ConversationId("relay:overflow-2".into());
    let mut third = test_inbound("bridge-overflow-3");
    third.conversation = ConversationId("relay:overflow-3".into());

    inbound_tx.send(first.clone()).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if event_rx.len() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    inbound_tx.send(second.clone()).await.unwrap();
    inbound_tx.send(third.clone()).await.unwrap();

    let mut pending = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            pending = store.pending_events_by_kind("message", 10).await.unwrap();
            if pending.len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let pending_ids = pending
        .iter()
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert!(pending_ids.contains(&inbound_event_id(&second).as_str()));
    assert!(pending_ids.contains(&inbound_event_id(&third).as_str()));

    match event_rx.try_recv().unwrap() {
        WakeEvent::Message(forwarded) => assert_eq!(forwarded.message_id, first.message_id),
        _ => panic!("expected first inbound message to be forwarded"),
    }
}
