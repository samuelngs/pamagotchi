use super::action::{ActionId, Outcome};
use crate::store::Store;
use protocol::{ConversationId, InboundMessage, PersonId};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FiredIntent {
    pub id: String,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub person: Option<PersonId>,
    #[serde(default)]
    pub scheduled_at: Option<i64>,
    #[serde(default)]
    pub chosen_person_approved: bool,
    pub defer_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageEditedEvent {
    pub conversation: ConversationId,
    pub gateway_id: String,
    pub message_id: String,
    pub content: String,
    pub edited_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageDeletedEvent {
    pub conversation: ConversationId,
    pub gateway_id: String,
    pub message_id: String,
    pub deleted_at: i64,
}

pub enum WakeEvent {
    Message(InboundMessage),
    IdleTick {
        elapsed_secs: f64,
    },
    ConsolidationDue,
    IntentFired(FiredIntent),
    TypingUpdate {
        conversation: ConversationId,
        gateway_id: String,
        sender_external_id: String,
        typing: bool,
    },
    MessageEdited {
        conversation: ConversationId,
        gateway_id: String,
        message_id: String,
        content: String,
        edited_at: i64,
    },
    MessageDeleted {
        conversation: ConversationId,
        gateway_id: String,
        message_id: String,
        deleted_at: i64,
    },
    ActionCompleted {
        action_id: ActionId,
        outcome: Outcome,
    },
    Shutdown,
}

impl WakeEvent {
    pub fn message(&self) -> Option<&InboundMessage> {
        match self {
            Self::Message(msg) => Some(msg),
            _ => None,
        }
    }
}

pub(crate) async fn claim_and_send_persisted_event(
    event_tx: &mpsc::Sender<WakeEvent>,
    store: &dyn Store,
    event_id: &str,
    fired_at: i64,
    wake: WakeEvent,
    context: &str,
) -> bool {
    let permit = match event_tx.reserve().await {
        Ok(permit) => permit,
        Err(_) => return false,
    };

    match store.mark_event_fired(event_id, fired_at).await {
        Ok(true) => {
            permit.send(wake);
        }
        Ok(false) => {
            debug!(
                event_id,
                context, "persisted wake event was already claimed"
            );
        }
        Err(e) => {
            warn!(
                %e,
                event_id,
                context,
                "failed to claim persisted wake event; leaving pending"
            );
        }
    }

    true
}
