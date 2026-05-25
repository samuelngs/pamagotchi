use crate::personality::{Authority, PersonalityDelta};
use crate::store::Thought;
use protocol::{ConversationId, InboundMessage, MemoryId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub String);

impl ActionId {
    pub fn new() -> Self {
        Self(nanoid::nanoid!())
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Respond => "respond",
            Self::Research => "research",
            Self::Consolidate => "consolidate",
            Self::Outreach => "outreach",
            Self::Ruminate => "ruminate",
        }
    }

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

pub struct ActionContext {
    pub cancelled_note: Option<String>,
    pub concurrent_actions: Vec<ActionBrief>,
}

pub struct ActionBrief {
    pub id: ActionId,
    pub kind: ActionKind,
    pub task: String,
    pub conversation: Option<ConversationId>,
}

pub struct ActionRequest {
    pub kind: ActionKind,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub priority: u8,
    pub messages: Vec<InboundMessage>,
    pub timing: ActionTiming,
    pub context: Option<ActionContext>,
    pub authority: Authority,
}

impl ActionRequest {
    pub fn respond(
        messages: Vec<InboundMessage>,
        conversation: ConversationId,
        authority: Authority,
    ) -> Self {
        Self {
            kind: ActionKind::Respond,
            task: "Respond to message".into(),
            conversation: Some(conversation),
            priority: ActionKind::Respond.default_priority(),
            messages,
            timing: ActionTiming::Immediate,
            context: None,
            authority,
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
            context: None,
            authority: Authority::Default,
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
            context: None,
            authority: Authority::Default,
        }
    }
}

pub struct ActionProgress {
    pub responded: bool,
    pub thoughts_count: usize,
    pub memories_formed: usize,
    pub last_activity: String,
}

impl ActionProgress {
    pub fn new() -> Self {
        Self {
            responded: false,
            thoughts_count: 0,
            memories_formed: 0,
            last_activity: String::new(),
        }
    }
}

pub struct ActionResult {
    pub delta: Option<PersonalityDelta>,
    pub thoughts: Vec<Thought>,
    pub memories_formed: Vec<MemoryId>,
    pub unprocessed_messages: Vec<InboundMessage>,
    pub injected_messages: Vec<InboundMessage>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum ActionStatus {
    Pending,
    Running,
    Completed,
    Cancelled,
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
    pub progress: Arc<RwLock<ActionProgress>>,
    pub inject_tx: Option<mpsc::Sender<InboundMessage>>,
}
