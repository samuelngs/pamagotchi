use super::*;

pub(super) fn inbound_bridge(
    event_tx: mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
) -> mpsc::Sender<InboundEnvelope> {
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<InboundEnvelope>(256);
    tokio::spawn(async move {
        while let Some(envelope) = inbound_rx.recv().await {
            let event_id_value = inbound_event_id(&envelope);
            let dedupe_key = inbound_event_dedupe_key(&envelope);
            let msg = match resolve_inbound_envelope(store.as_ref(), envelope).await {
                Ok(msg) => msg,
                Err(e) => {
                    warn!(%e, "failed to resolve inbound envelope");
                    continue;
                }
            };
            let event_id =
                persist_inbound_message_event(store.as_ref(), &event_id_value, dedupe_key, &msg)
                    .await;
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

async fn resolve_inbound_envelope(
    store: &dyn Store,
    envelope: InboundEnvelope,
) -> anyhow::Result<InboundMessage> {
    envelope
        .validate()
        .map_err(|message| anyhow::anyhow!(message))?;
    let now = envelope.timestamp;
    store
        .upsert_gateway(&GatewayRecord {
            id: envelope.gateway_id.clone(),
            kind: gateway_kind_from_envelope(&envelope),
            display_name: None,
            metadata: serde_json::json!({
                "source": "inbound_envelope",
            }),
            created_at: now,
            updated_at: now,
        })
        .await?;

    let conversation =
        resolve_channel_conversation(store, &envelope.gateway_id, &envelope.channel, now).await?;

    Ok(inbound_message_from_envelope(envelope, conversation))
}

async fn resolve_channel_conversation(
    store: &dyn Store,
    gateway_id: &protocol::GatewayId,
    channel: &protocol::ChannelKey,
    now: i64,
) -> anyhow::Result<ConversationId> {
    if &channel.gateway_id != gateway_id {
        anyhow::bail!(
            "channel gateway_id mismatch: expected {}, got {}",
            gateway_id.0,
            channel.gateway_id.0
        );
    }
    if let Some(space) = channel.space.as_ref() {
        store.upsert_space(&space_record(space, now)).await?;
    }

    let parent_id = if let Some(parent) = channel.parent.as_ref() {
        if &parent.gateway_id != gateway_id {
            anyhow::bail!(
                "parent channel gateway_id mismatch: expected {}, got {}",
                gateway_id.0,
                parent.gateway_id.0
            );
        }
        if let Some(space) = parent.space.as_ref() {
            store.upsert_space(&space_record(space, now)).await?;
        }
        let parent_id = protocol::channel_id(&parent.gateway_id, &parent.external_id);
        let parent_space = parent
            .space
            .as_ref()
            .map(|space| protocol::space_id(&space.gateway_id, &space.external_id));
        store
            .upsert_channel(&ChannelRecord {
                id: parent_id.clone(),
                gateway: parent.gateway_id.clone(),
                external_id: parent.external_id.clone(),
                kind: parent.kind.clone(),
                space: parent_space,
                parent: None,
                display_name: parent.display_name.clone(),
                metadata: parent.metadata.clone(),
                created_at: now,
                updated_at: now,
                last_seen_at: now,
            })
            .await?;
        Some(parent_id)
    } else {
        None
    };

    let channel_id = protocol::channel_id(&channel.gateway_id, &channel.external_id);
    let space_id = channel
        .space
        .as_ref()
        .map(|space| protocol::space_id(&space.gateway_id, &space.external_id));
    store
        .upsert_channel(&ChannelRecord {
            id: channel_id.clone(),
            gateway: channel.gateway_id.clone(),
            external_id: channel.external_id.clone(),
            kind: channel.kind.clone(),
            space: space_id,
            parent: parent_id,
            display_name: channel.display_name.clone(),
            metadata: channel.metadata.clone(),
            created_at: now,
            updated_at: now,
            last_seen_at: now,
        })
        .await?;
    store
        .get_or_create_active_conversation(&channel_id, now)
        .await
}

async fn resolve_runtime_channel_conversation(
    store: &dyn Store,
    gateway_id: &protocol::GatewayId,
    channel: &protocol::ChannelKey,
    now: i64,
) -> anyhow::Result<ConversationId> {
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway_id.clone(),
            kind: channel
                .metadata
                .get("platform")
                .and_then(|value| value.as_str())
                .unwrap_or(gateway_id.as_str())
                .to_string(),
            display_name: None,
            metadata: serde_json::json!({
                "source": "gateway_runtime_event",
            }),
            created_at: now,
            updated_at: now,
        })
        .await?;
    resolve_channel_conversation(store, gateway_id, channel, now).await
}

fn inbound_message_from_envelope(
    envelope: InboundEnvelope,
    conversation: ConversationId,
) -> InboundMessage {
    let mut metadata = envelope.metadata.clone();
    let normalized = serde_json::to_value(&envelope).unwrap_or(serde_json::Value::Null);
    match &mut metadata {
        serde_json::Value::Object(obj) => {
            obj.insert("normalized_envelope".into(), normalized);
        }
        serde_json::Value::Null => {
            metadata = serde_json::json!({
                "normalized_envelope": normalized,
            });
        }
        other => {
            metadata = serde_json::json!({
                "source_metadata": other.clone(),
                "normalized_envelope": normalized,
            });
        }
    }

    InboundMessage {
        message_id: envelope.platform_message_id,
        gateway_id: envelope.gateway_id.0.clone(),
        sender: envelope.sender,
        channel: envelope.channel,
        conversation,
        identity: None,
        profile: None,
        person: None,
        content: envelope.content,
        attachments: envelope.attachments,
        timestamp: envelope.timestamp,
        metadata,
    }
}

fn gateway_kind_from_envelope(envelope: &InboundEnvelope) -> String {
    envelope
        .channel
        .metadata
        .get("platform")
        .and_then(|value| value.as_str())
        .or_else(|| {
            envelope
                .metadata
                .get("platform")
                .and_then(|value| value.as_str())
        })
        .unwrap_or(envelope.gateway_id.as_str())
        .to_string()
}

fn space_record(space: &protocol::SpaceKey, now: i64) -> SpaceRecord {
    SpaceRecord {
        id: protocol::space_id(&space.gateway_id, &space.external_id),
        gateway: space.gateway_id.clone(),
        external_id: space.external_id.clone(),
        kind: space.kind.clone(),
        display_name: space.display_name.clone(),
        metadata: space.metadata.clone(),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    }
}

async fn persist_inbound_message_event(
    store: &dyn Store,
    event_id: &str,
    dedupe_key: Option<String>,
    msg: &InboundMessage,
) -> Option<String> {
    let now = now_secs();
    let record = EventInboxRecord {
        id: event_id.to_string(),
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
        dedupe_key,
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id.to_string()),
        Err(e) => {
            warn!(%e, message_id = %msg.message_id, "failed to persist inbound message event");
            None
        }
    }
}

pub(super) fn inbound_event_id(msg: &InboundEnvelope) -> String {
    if !msg.gateway_id.0.is_empty() && !msg.platform_message_id.is_empty() {
        format!(
            "event-inbound:{}:{}",
            msg.gateway_id, msg.platform_message_id
        )
    } else {
        format!("event-inbound:{}:{}", now_millis(), rand::random::<u64>())
    }
}

fn inbound_event_dedupe_key(msg: &InboundEnvelope) -> Option<String> {
    (!msg.gateway_id.0.is_empty() && !msg.platform_message_id.is_empty()).then(|| {
        format!(
            "inbound-message:{}:{}",
            msg.gateway_id, msg.platform_message_id
        )
    })
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
                    channel,
                    sender,
                    typing,
                } => {
                    let now = now_secs();
                    let conversation = match resolve_runtime_channel_conversation(
                        store.as_ref(),
                        &gateway_id,
                        &channel,
                        now,
                    )
                    .await
                    {
                        Ok(conversation) => conversation,
                        Err(e) => {
                            warn!(%e, gateway = %gateway_id.0, "failed to resolve gateway typing channel");
                            continue;
                        }
                    };
                    if actor_event_tx
                        .send(WakeEvent::TypingUpdate {
                            conversation,
                            gateway_id: gateway_id.0,
                            sender_external_id: sender.external_id,
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
                    channel,
                    platform_message_id,
                    content,
                    edited_at,
                } => {
                    let conversation = match resolve_runtime_channel_conversation(
                        store.as_ref(),
                        &gateway_id,
                        &channel,
                        edited_at,
                    )
                    .await
                    {
                        Ok(conversation) => conversation,
                        Err(e) => {
                            warn!(%e, gateway = %gateway_id.0, message_id = %platform_message_id, "failed to resolve gateway message edit channel");
                            continue;
                        }
                    };
                    let edited = MessageEditedEvent {
                        conversation,
                        gateway_id: gateway_id.0,
                        message_id: platform_message_id,
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
                    channel,
                    platform_message_id,
                    deleted_at,
                } => {
                    let conversation = match resolve_runtime_channel_conversation(
                        store.as_ref(),
                        &gateway_id,
                        &channel,
                        deleted_at,
                    )
                    .await
                    {
                        Ok(conversation) => conversation,
                        Err(e) => {
                            warn!(%e, gateway = %gateway_id.0, message_id = %platform_message_id, "failed to resolve gateway message delete channel");
                            continue;
                        }
                    };
                    let deleted = MessageDeletedEvent {
                        conversation,
                        gateway_id: gateway_id.0,
                        message_id: platform_message_id,
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
