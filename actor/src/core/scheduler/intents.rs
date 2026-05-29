use crate::core::event::{FiredIntent, WakeEvent};
use crate::store::{IntentRecord, Store};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error};

pub(crate) async fn drain_due_intents(
    event_tx: &mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
    now: i64,
    limit: usize,
) -> bool {
    let intents = match store.due_intents(now, limit).await {
        Ok(intents) => intents,
        Err(e) => {
            error!(%e, "failed to scan due intents");
            return true;
        }
    };

    for intent in intents {
        if !claim_and_send_due_intent(event_tx, store.as_ref(), intent, now).await {
            return false;
        }
    }

    true
}

pub(crate) async fn claim_and_send_due_intent(
    event_tx: &mpsc::Sender<WakeEvent>,
    store: &dyn Store,
    intent: IntentRecord,
    fired_at: i64,
) -> bool {
    let intent_id = intent.id.clone();
    let permit = match event_tx.reserve().await {
        Ok(permit) => permit,
        Err(_) => return false,
    };

    let wake = WakeEvent::IntentFired(FiredIntent {
        id: intent.id,
        task: intent.task,
        conversation: intent.conversation,
        person: intent.person,
        scheduled_at: Some(intent.updated_at.max(intent.created_at)),
        chosen_human_approved: intent.chosen_human_approved,
        defer_count: 0,
    });

    match store.mark_intent_fired(&intent_id, fired_at).await {
        Ok(true) => {
            permit.send(wake);
        }
        Ok(false) => {
            debug!(
                intent_id = %intent_id,
                "due intent was already claimed or no longer active"
            );
        }
        Err(e) => {
            error!(
                %e,
                intent_id = %intent_id,
                "failed to claim due intent; leaving active"
            );
        }
    }

    true
}
