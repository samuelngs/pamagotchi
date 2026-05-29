use super::*;

fn relay_channel(external_id: &str) -> protocol::ChannelKey {
    protocol::ChannelKey {
        gateway_id: protocol::GatewayId("relay".into()),
        external_id: external_id.into(),
        kind: protocol::ChannelKind::RelayRoom,
        display_name: None,
        space: None,
        parent: None,
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn gateway_typing_event_uses_store_resolved_channel_conversation() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (api, _api_rx) = ApiServer::listen(0).await.unwrap();
    let (gateway_event_tx, gateway_event_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = mpsc::channel(1);
    spawn_gateway_event_listener(gateway_event_rx, api.handle(), event_tx, store_dyn);

    gateway_event_tx
        .send(GatewayRuntimeEvent::TypingUpdate {
            gateway_id: protocol::GatewayId("relay".into()),
            channel: relay_channel("local"),
            sender: protocol::ObservedIdentityKey {
                gateway_id: protocol::GatewayId("relay".into()),
                external_id: "sender-1".into(),
                kind: Some("relay_user".into()),
                confidence: 1.0,
                source: "typing_sender".into(),
            },
            typing: true,
        })
        .await
        .unwrap();

    match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap()
    {
        WakeEvent::TypingUpdate {
            conversation,
            gateway_id,
            sender_external_id,
            typing,
        } => {
            assert_eq!(gateway_id, "relay");
            assert_eq!(sender_external_id, "sender-1");
            assert!(typing);
            assert_ne!(conversation, ConversationId("relay:local".into()));
            let channel = store
                .resolve_channel(&protocol::GatewayId("relay".into()), "local")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                store
                    .channel_for_conversation(&conversation)
                    .await
                    .unwrap()
                    .unwrap()
                    .id,
                channel.id
            );
        }
        _ => panic!("expected typing event"),
    }
}

#[tokio::test]
async fn gateway_event_listener_persists_and_forwards_message_revisions() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (api, _api_rx) = ApiServer::listen(0).await.unwrap();
    let (gateway_event_tx, gateway_event_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = mpsc::channel(2);
    spawn_gateway_event_listener(gateway_event_rx, api.handle(), event_tx, store_dyn);

    gateway_event_tx
        .send(GatewayRuntimeEvent::MessageEdited {
            gateway_id: protocol::GatewayId("relay".into()),
            channel: relay_channel("local"),
            platform_message_id: "revision-msg-1".into(),
            content: "edited content".into(),
            edited_at: 1100,
        })
        .await
        .unwrap();
    gateway_event_tx
        .send(GatewayRuntimeEvent::MessageDeleted {
            gateway_id: protocol::GatewayId("relay".into()),
            channel: relay_channel("local"),
            platform_message_id: "revision-msg-2".into(),
            deleted_at: 1200,
        })
        .await
        .unwrap();

    let edited_conversation =
        match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WakeEvent::MessageEdited {
                conversation,
                gateway_id,
                message_id,
                content,
                edited_at,
            } => {
                assert_eq!(gateway_id, "relay");
                assert_eq!(message_id, "revision-msg-1");
                assert_eq!(content, "edited content");
                assert_eq!(edited_at, 1100);
                assert_ne!(conversation, ConversationId("relay:local".into()));
                conversation
            }
            _ => panic!("expected message edit event"),
        };
    let deleted_conversation =
        match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WakeEvent::MessageDeleted {
                conversation,
                gateway_id,
                message_id,
                deleted_at,
            } => {
                assert_eq!(gateway_id, "relay");
                assert_eq!(message_id, "revision-msg-2");
                assert_eq!(deleted_at, 1200);
                assert_eq!(conversation, edited_conversation);
                conversation
            }
            _ => panic!("expected message delete event"),
        };

    let edited = MessageEditedEvent {
        conversation: edited_conversation,
        gateway_id: "relay".into(),
        message_id: "revision-msg-1".into(),
        content: "edited content".into(),
        edited_at: 1100,
    };
    let deleted = MessageDeletedEvent {
        conversation: deleted_conversation,
        gateway_id: "relay".into(),
        message_id: "revision-msg-2".into(),
        deleted_at: 1200,
    };
    assert!(
        !store
            .mark_event_fired(&message_edited_event_id(&edited), now_secs())
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_event_fired(&message_deleted_event_id(&deleted), now_secs())
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn gateway_event_listener_leaves_revision_pending_when_actor_channel_is_closed() {
    let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
    let store_dyn: Arc<dyn Store> = store.clone();
    let (api, _api_rx) = ApiServer::listen(0).await.unwrap();
    let (gateway_event_tx, gateway_event_rx) = mpsc::channel(1);
    let (event_tx, event_rx) = mpsc::channel(1);
    drop(event_rx);
    spawn_gateway_event_listener(gateway_event_rx, api.handle(), event_tx, store_dyn);

    gateway_event_tx
        .send(GatewayRuntimeEvent::MessageEdited {
            gateway_id: protocol::GatewayId("relay".into()),
            channel: relay_channel("local"),
            platform_message_id: "pending-revision-msg".into(),
            content: "edited content".into(),
            edited_at: 1100,
        })
        .await
        .unwrap();

    let mut due = Vec::new();
    for _ in 0..20 {
        due = store.due_events(now_secs() + 1, 10).await.unwrap();
        if !due.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let channel = store
        .resolve_channel(&protocol::GatewayId("relay".into()), "local")
        .await
        .unwrap()
        .unwrap();
    let conversation = store
        .get_or_create_active_conversation(&channel.id, 1100)
        .await
        .unwrap();
    let edited = MessageEditedEvent {
        conversation,
        gateway_id: "relay".into(),
        message_id: "pending-revision-msg".into(),
        content: "edited content".into(),
        edited_at: 1100,
    };

    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, message_edited_event_id(&edited));
    assert_eq!(due[0].kind, "message_edited");
    assert_eq!(due[0].status, "pending");
}
