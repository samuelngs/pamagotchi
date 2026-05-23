use crate::personality::PersonalityDelta;
use crate::store::{ConversationId, MemoryId, Thought};
use super::event::InboundMessage;
use serde::{Deserialize, Serialize};
use std::fmt;
use tokio::task::JoinHandle;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub u64);

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "action-{}", self.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActionKind {
    Respond,
    Research,
    Consolidate,
    Outreach,
    Ruminate,
}

impl ActionKind {
    pub fn default_priority(&self) -> u8 {
        match self {
            Self::Respond => 80,
            Self::Research => 60,
            Self::Outreach => 50,
            Self::Consolidate => 20,
            Self::Ruminate => 10,
        }
    }
}

pub enum ActionTiming {
    Immediate,
    After(ActionId),
    AfterAll(Vec<ActionId>),
}

pub struct ActionRequest {
    pub kind: ActionKind,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub priority: u8,
    pub messages: Vec<InboundMessage>,
    pub timing: ActionTiming,
}

impl ActionRequest {
    pub fn respond(messages: Vec<InboundMessage>, conversation: ConversationId) -> Self {
        Self {
            kind: ActionKind::Respond,
            task: "Respond to message".into(),
            conversation: Some(conversation),
            priority: ActionKind::Respond.default_priority(),
            messages,
            timing: ActionTiming::Immediate,
        }
    }

    pub fn ruminate() -> Self {
        Self {
            kind: ActionKind::Ruminate,
            task: "Idle rumination".into(),
            conversation: None,
            priority: ActionKind::Ruminate.default_priority(),
            messages: vec![],
            timing: ActionTiming::Immediate,
        }
    }

    pub fn consolidate() -> Self {
        Self {
            kind: ActionKind::Consolidate,
            task: "Memory consolidation".into(),
            conversation: None,
            priority: ActionKind::Consolidate.default_priority(),
            messages: vec![],
            timing: ActionTiming::Immediate,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum ActionStatus {
    Pending,
    Running,
    Completed,
    Cancelled,
}

pub struct ActionResult {
    pub delta: Option<PersonalityDelta>,
    pub thoughts: Vec<Thought>,
    pub memories_formed: Vec<MemoryId>,
}

pub struct ActionState {
    pub id: ActionId,
    pub kind: ActionKind,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub priority: u8,
    pub status: ActionStatus,
    pub has_responded: bool,
    pub depends_on: Vec<ActionId>,
    pub handle: Option<JoinHandle<()>>,
}

pub struct MindDecision {
    pub spawn: Vec<ActionRequest>,
    pub cancel: Vec<ActionId>,
    pub supplement: Vec<(ActionId, SupplementContext)>,
}

impl MindDecision {
    pub fn drop() -> Self {
        Self {
            spawn: vec![],
            cancel: vec![],
            supplement: vec![],
        }
    }

    pub fn spawn_one(request: ActionRequest) -> Self {
        Self {
            spawn: vec![request],
            cancel: vec![],
            supplement: vec![],
        }
    }

    pub fn cancel_and_spawn(cancel: Vec<ActionId>, request: ActionRequest) -> Self {
        Self {
            spawn: vec![request],
            cancel,
            supplement: vec![],
        }
    }
}

pub struct SupplementContext {
    pub messages: Vec<InboundMessage>,
    pub note: String,
}
