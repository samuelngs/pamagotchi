use crate::identity::{GroupId, PersonId};
use crate::store::ConversationId;
use super::action::{ActionId, ActionResult};

#[derive(Clone, Debug)]
pub struct InboundMessage {
    pub platform_id: String,
    pub external_id: String,
    pub conversation: ConversationId,
    pub group: Option<GroupId>,
    pub person: Option<PersonId>,
    pub content: String,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct FiredIntent {
    pub id: String,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub person: Option<PersonId>,
}

pub enum WakeEvent {
    Message(InboundMessage),
    IdleTick { elapsed_secs: f64 },
    IntentFired(FiredIntent),
    TypingUpdate {
        conversation: ConversationId,
        person: PersonId,
        typing: bool,
    },
    ActionCompleted {
        action_id: ActionId,
        result: ActionResult,
    },
    Shutdown,
}
