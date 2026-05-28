use super::*;

pub(super) fn inbound_bridge(
    event_tx: mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
) -> mpsc::Sender<InboundMessage> {
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<InboundMessage>(256);
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            let event_id = persist_inbound_message_event(store.as_ref(), &msg).await;
            let permit = match event_id {
                Some(_) => {
                    match tokio::time::timeout(INBOUND_ACTOR_HANDOFF_TIMEOUT, event_tx.reserve())
                        .await
                    {
                        Ok(Ok(permit)) => permit,
                        Ok(Err(_)) => break,
                        Err(_) => {
                            warn!(
                                gateway = %msg.gateway_id,
                                message_id = %msg.message_id,
                                "actor event channel full; inbound message remains pending"
                            );
                            continue;
                        }
                    }
                }
                None => match event_tx.reserve().await {
                    Ok(permit) => permit,
                    Err(_) => break,
                },
            };
            if let Some(event_id) = event_id {
                match store.mark_event_fired(&event_id, now_secs()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        debug!(
                            event_id = %event_id,
                            message_id = %msg.message_id,
                            "inbound message event was already claimed"
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            %e,
                            event_id = %event_id,
                            message_id = %msg.message_id,
                            "failed to claim inbound message event; forwarding directly"
                        );
                    }
                }
            }
            permit.send(WakeEvent::Message(msg));
        }
    });
    inbound_tx
}

async fn persist_inbound_message_event(store: &dyn Store, msg: &InboundMessage) -> Option<String> {
    let now = now_secs();
    let event_id = inbound_event_id(msg);
    let record = EventInboxRecord {
        id: event_id.clone(),
        kind: "message".into(),
        payload: match serde_json::to_value(msg) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, message_id = %msg.message_id, "failed to serialize inbound message event");
                return None;
            }
        },
        status: "pending".into(),
        due_at: now,
        attempts: 0,
        dedupe_key: inbound_event_dedupe_key(msg),
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id),
        Err(e) => {
            warn!(%e, message_id = %msg.message_id, "failed to persist inbound message event");
            None
        }
    }
}

pub(super) fn inbound_event_id(msg: &InboundMessage) -> String {
    if !msg.gateway_id.is_empty() && !msg.message_id.is_empty() {
        format!("event-inbound:{}:{}", msg.gateway_id, msg.message_id)
    } else {
        format!("event-inbound:{}:{}", now_millis(), rand::random::<u64>())
    }
}

fn inbound_event_dedupe_key(msg: &InboundMessage) -> Option<String> {
    (!msg.gateway_id.is_empty() && !msg.message_id.is_empty())
        .then(|| format!("inbound-message:{}:{}", msg.gateway_id, msg.message_id))
}

pub(super) fn spawn_gateway_event_listener(
    mut gateway_event_rx: mpsc::Receiver<GatewayRuntimeEvent>,
    api_handle: ApiServerHandle,
    actor_event_tx: mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
) {
    tokio::spawn(async move {
        while let Some(event) = gateway_event_rx.recv().await {
            match event {
                GatewayRuntimeEvent::ConnectionStateChanged { gateway_id, state } => {
                    info!(gateway = %gateway_id, ?state, "gateway connection state changed");
                    api_handle
                        .broadcast(ServerEvent::GatewayConnectionStateChanged {
                            id: gateway_id,
                            state,
                        })
                        .await;
                }
                GatewayRuntimeEvent::SetupInstructionsChanged { gateway_id, setup } => {
                    info!(
                        gateway = %gateway_id,
                        has_setup = setup.is_some(),
                        "gateway setup instructions changed"
                    );
                    api_handle
                        .broadcast(ServerEvent::GatewaySetupInstructionsChanged {
                            id: gateway_id,
                            setup,
                        })
                        .await;
                }
                GatewayRuntimeEvent::TypingUpdate {
                    gateway_id,
                    conversation,
                    sender_external_id,
                    typing,
                } => {
                    if actor_event_tx
                        .send(WakeEvent::TypingUpdate {
                            conversation,
                            gateway_id,
                            sender_external_id,
                            typing,
                        })
                        .await
                        .is_err()
                    {
                        warn!("failed to forward gateway typing event to actor");
                    }
                }
                GatewayRuntimeEvent::MessageEdited {
                    gateway_id,
                    conversation,
                    message_id,
                    content,
                    edited_at,
                } => {
                    let edited = MessageEditedEvent {
                        conversation,
                        gateway_id,
                        message_id,
                        content,
                        edited_at,
                    };
                    let event_id = persist_message_edited_event(store.as_ref(), &edited).await;
                    let wake = WakeEvent::MessageEdited {
                        conversation: edited.conversation,
                        gateway_id: edited.gateway_id,
                        message_id: edited.message_id,
                        content: edited.content,
                        edited_at: edited.edited_at,
                    };
                    if !forward_persisted_gateway_event(
                        &actor_event_tx,
                        store.as_ref(),
                        event_id,
                        wake,
                        "gateway message edit",
                    )
                    .await
                    {
                        warn!("failed to forward gateway message edit event to actor");
                    }
                }
                GatewayRuntimeEvent::MessageDeleted {
                    gateway_id,
                    conversation,
                    message_id,
                    deleted_at,
                } => {
                    let deleted = MessageDeletedEvent {
                        conversation,
                        gateway_id,
                        message_id,
                        deleted_at,
                    };
                    let event_id = persist_message_deleted_event(store.as_ref(), &deleted).await;
                    let wake = WakeEvent::MessageDeleted {
                        conversation: deleted.conversation,
                        gateway_id: deleted.gateway_id,
                        message_id: deleted.message_id,
                        deleted_at: deleted.deleted_at,
                    };
                    if !forward_persisted_gateway_event(
                        &actor_event_tx,
                        store.as_ref(),
                        event_id,
                        wake,
                        "gateway message delete",
                    )
                    .await
                    {
                        warn!("failed to forward gateway message delete event to actor");
                    }
                }
            }
        }
    });
}

async fn persist_message_edited_event(
    store: &dyn Store,
    event: &MessageEditedEvent,
) -> Option<String> {
    let now = now_secs();
    let event_id = message_edited_event_id(event);
    let record = EventInboxRecord {
        id: event_id.clone(),
        kind: "message_edited".into(),
        payload: match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, message_id = %event.message_id, "failed to serialize message edit event");
                return None;
            }
        },
        status: "pending".into(),
        due_at: now,
        attempts: 0,
        dedupe_key: Some(message_edited_event_dedupe_key(event)),
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id),
        Err(e) => {
            warn!(%e, message_id = %event.message_id, "failed to persist message edit event");
            None
        }
    }
}

async fn persist_message_deleted_event(
    store: &dyn Store,
    event: &MessageDeletedEvent,
) -> Option<String> {
    let now = now_secs();
    let event_id = message_deleted_event_id(event);
    let record = EventInboxRecord {
        id: event_id.clone(),
        kind: "message_deleted".into(),
        payload: match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, message_id = %event.message_id, "failed to serialize message delete event");
                return None;
            }
        },
        status: "pending".into(),
        due_at: now,
        attempts: 0,
        dedupe_key: Some(message_deleted_event_dedupe_key(event)),
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id),
        Err(e) => {
            warn!(%e, message_id = %event.message_id, "failed to persist message delete event");
            None
        }
    }
}

async fn forward_persisted_gateway_event(
    actor_event_tx: &mpsc::Sender<WakeEvent>,
    store: &dyn Store,
    event_id: Option<String>,
    wake: WakeEvent,
    context: &str,
) -> bool {
    let Some(event_id) = event_id else {
        return actor_event_tx.send(wake).await.is_ok();
    };

    let permit =
        match tokio::time::timeout(INBOUND_ACTOR_HANDOFF_TIMEOUT, actor_event_tx.reserve()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return false,
            Err(_) => {
                warn!(
                    event_id = %event_id,
                    context,
                    "actor event channel full; gateway event remains pending"
                );
                return true;
            }
        };

    match store.mark_event_fired(&event_id, now_secs()).await {
        Ok(true) => permit.send(wake),
        Ok(false) => {
            debug!(
                event_id = %event_id,
                context, "gateway event was already claimed"
            );
        }
        Err(e) => {
            warn!(
                %e,
                event_id = %event_id,
                context, "failed to claim gateway event; forwarding directly"
            );
            permit.send(wake);
        }
    }
    true
}

pub(super) fn message_edited_event_id(event: &MessageEditedEvent) -> String {
    format!(
        "event-message-edit:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.edited_at
    )
}

fn message_edited_event_dedupe_key(event: &MessageEditedEvent) -> String {
    format!(
        "message-edit:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.edited_at
    )
}

pub(super) fn message_deleted_event_id(event: &MessageDeletedEvent) -> String {
    format!(
        "event-message-delete:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.deleted_at
    )
}

fn message_deleted_event_dedupe_key(event: &MessageDeletedEvent) -> String {
    format!(
        "message-delete:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.deleted_at
    )
}
