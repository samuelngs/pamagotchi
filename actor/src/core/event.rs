use crate::identity::{GroupId, PersonId};
use crate::platform::MediaAttachment;
use crate::store::ConversationId;
use super::action::{ActionId, ActionResult};

#[derive(Clone, Debug)]
pub struct InboundMessage {
    pub message_id: String,
    pub platform_id: String,
    pub external_id: String,
    pub conversation: ConversationId,
    pub group: Option<GroupId>,
    pub person: Option<PersonId>,
    pub content: String,
    pub media: Option<MediaAttachment>,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

impl InboundMessage {
    pub fn display_content(&self) -> String {
        match &self.media {
            None => self.content.clone(),
            Some(media) => {
                let label = media.kind.label();
                match &media.filename {
                    Some(fname) if self.content.is_empty() => format!("[{label}: {fname}]"),
                    Some(fname) => format!("[{label}: {fname}] {}", self.content),
                    None if self.content.is_empty() => format!("[{label}]"),
                    None => format!("[{label}] {}", self.content),
                }
            }
        }
    }
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

impl WakeEvent {
    pub fn message(&self) -> Option<&InboundMessage> {
        match self {
            Self::Message(msg) => Some(msg),
            _ => None,
        }
    }
}
