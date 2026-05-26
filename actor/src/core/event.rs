use super::action::{ActionId, Outcome};
use protocol::{ConversationId, InboundMessage, PersonId};

#[derive(Clone, Debug)]
pub struct FiredIntent {
    pub id: String,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub person: Option<PersonId>,
}

pub enum WakeEvent {
    Message(InboundMessage),
    IdleTick {
        elapsed_secs: f64,
    },
    IntentFired(FiredIntent),
    TypingUpdate {
        conversation: ConversationId,
        person: PersonId,
        typing: bool,
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
