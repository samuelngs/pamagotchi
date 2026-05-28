use crate::core::event::{
    MessageDeletedEvent, MessageEditedEvent, WakeEvent, claim_and_send_persisted_event,
};
use crate::store::{EventInboxRecord, Store};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::error;

pub(crate) async fn emit_due_consolidation(
    event_tx: &mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
    now: i64,
) -> bool {
    match store.pending_events_by_kind("consolidation_due", 1).await {
        Ok(pending) if !pending.is_empty() => return true,
        Ok(_) => {}
        Err(e) => {
            error!(%e, "failed to check pending consolidation events");
            return true;
        }
    }

    let event_id = format!("event-consolidation-{}", nanoid::nanoid!());
    if let Err(e) = store
        .enqueue_event(&EventInboxRecord {
            id: event_id.clone(),
            kind: "consolidation_due".into(),
            payload: serde_json::json!({}),
            status: "pending".into(),
            due_at: now,
            attempts: 0,
            dedupe_key: Some("consolidation-due".into()),
            created_at: now,
            updated_at: now,
            fired_at: None,
            last_error: None,
        })
        .await
    {
        error!(%e, "failed to persist periodic consolidation event");
        return true;
    }

    claim_and_send_persisted_event(
        event_tx,
        store.as_ref(),
        &event_id,
        now,
        WakeEvent::ConsolidationDue,
        "periodic consolidation",
    )
    .await
}

pub(crate) async fn drain_due_events(
    event_tx: &mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
    now: i64,
    limit: usize,
) -> bool {
    let events = match store.due_events(now, limit).await {
        Ok(events) => events,
        Err(e) => {
            error!(%e, "failed to scan due event inbox");
            return true;
        }
    };

    for event in events {
        let wake = match event.kind.as_str() {
            "message" => match serde_json::from_value(event.payload.clone()) {
                Ok(message) => WakeEvent::Message(message),
                Err(e) => {
                    error!(%e, event_id = %event.id, "failed to deserialize deferred message");
                    let error = deferred_event_error(
                        event.last_error.as_deref(),
                        format!("failed to deserialize deferred message: {e}"),
                    );
                    mark_deferred_event_failed(store.as_ref(), &event.id, now, &error).await;
                    continue;
                }
            },
            "intent_fired" => match serde_json::from_value(event.payload.clone()) {
                Ok(intent) => WakeEvent::IntentFired(intent),
                Err(e) => {
                    error!(%e, event_id = %event.id, "failed to deserialize deferred intent");
                    let error = deferred_event_error(
                        event.last_error.as_deref(),
                        format!("failed to deserialize deferred intent: {e}"),
                    );
                    mark_deferred_event_failed(store.as_ref(), &event.id, now, &error).await;
                    continue;
                }
            },
            "message_edited" => {
                match serde_json::from_value::<MessageEditedEvent>(event.payload.clone()) {
                    Ok(edited) => WakeEvent::MessageEdited {
                        conversation: edited.conversation,
                        gateway_id: edited.gateway_id,
                        message_id: edited.message_id,
                        content: edited.content,
                        edited_at: edited.edited_at,
                    },
                    Err(e) => {
                        error!(%e, event_id = %event.id, "failed to deserialize message edit event");
                        let error = deferred_event_error(
                            event.last_error.as_deref(),
                            format!("failed to deserialize message edit event: {e}"),
                        );
                        mark_deferred_event_failed(store.as_ref(), &event.id, now, &error).await;
                        continue;
                    }
                }
            }
            "message_deleted" => {
                match serde_json::from_value::<MessageDeletedEvent>(event.payload.clone()) {
                    Ok(deleted) => WakeEvent::MessageDeleted {
                        conversation: deleted.conversation,
                        gateway_id: deleted.gateway_id,
                        message_id: deleted.message_id,
                        deleted_at: deleted.deleted_at,
                    },
                    Err(e) => {
                        error!(%e, event_id = %event.id, "failed to deserialize message delete event");
                        let error = deferred_event_error(
                            event.last_error.as_deref(),
                            format!("failed to deserialize message delete event: {e}"),
                        );
                        mark_deferred_event_failed(store.as_ref(), &event.id, now, &error).await;
                        continue;
                    }
                }
            }
            "consolidation_due" => WakeEvent::ConsolidationDue,
            other => {
                error!(event_id = %event.id, kind = %other, "unknown deferred event kind");
                let error = deferred_event_error(
                    event.last_error.as_deref(),
                    format!("unknown deferred event kind: {other}"),
                );
                mark_deferred_event_failed(store.as_ref(), &event.id, now, &error).await;
                continue;
            }
        };

        if !claim_and_send_persisted_event(
            event_tx,
            store.as_ref(),
            &event.id,
            now,
            wake,
            "scheduler due event",
        )
        .await
        {
            return false;
        }
    }

    true
}

fn deferred_event_error(prior: Option<&str>, error: String) -> String {
    match prior {
        Some(prior) if !prior.is_empty() => format!("{prior}; {error}"),
        _ => error,
    }
}

async fn mark_deferred_event_failed(store: &dyn Store, id: &str, failed_at: i64, error: &str) {
    match store.mark_event_failed(id, failed_at, Some(error)).await {
        Ok(true) | Ok(false) => {}
        Err(e) => error!(%e, event_id = %id, "failed to mark deferred event failed"),
    }
}
