use crate::state::{Authority, Delta};
use protocol::{ConversationId, InboundMessage};
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

#[derive(Clone, Debug)]
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

    pub fn expects_response(&self) -> bool {
        matches!(self, Self::Respond | Self::Outreach)
    }
}

pub struct Action {
    pub id: ActionId,
    pub kind: ActionKind,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub priority: u8,
    pub authority: Authority,
    pub style_directive: Option<String>,
    pub cancelled_note: Option<String>,
    pub source_messages: Vec<InboundMessage>,
    pub phase: Phase,
}

pub enum Phase {
    Queued {
        blocked_by: Vec<ActionId>,
    },
    Running {
        handle: Option<JoinHandle<()>>,
        inject_tx: mpsc::Sender<InboundMessage>,
        progress: Arc<RwLock<RunningState>>,
    },
    Done {
        outcome: Outcome,
    },
}

pub struct RunningState {
    pub responded: bool,
    pub last_tool: String,
}

impl RunningState {
    pub fn new() -> Self {
        Self {
            responded: false,
            last_tool: String::new(),
        }
    }
}

pub struct Outcome {
    pub responded: bool,
    pub delta: Option<Delta>,
    pub pending_messages: Vec<InboundMessage>,
    pub had_injections: bool,
}

pub struct LaunchContext {
    pub messages: Vec<InboundMessage>,
    pub inject_rx: mpsc::Receiver<InboundMessage>,
    pub progress: Arc<RwLock<RunningState>>,
}

pub enum FollowUp {
    Requeue(Vec<InboundMessage>),
    ReemitPending(Vec<InboundMessage>),
}

impl Action {
    pub fn respond(
        messages: Vec<InboundMessage>,
        conversation: ConversationId,
        authority: Authority,
        style_directive: Option<String>,
    ) -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Respond,
            task: "Respond to message".into(),
            conversation: Some(conversation),
            priority: ActionKind::Respond.default_priority(),
            authority,
            style_directive,
            cancelled_note: None,
            source_messages: messages,
            phase: Phase::Queued { blocked_by: vec![] },
        }
    }

    pub fn ruminate() -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Ruminate,
            task: "Idle rumination".into(),
            conversation: None,
            priority: ActionKind::Ruminate.default_priority(),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            source_messages: vec![],
            phase: Phase::Queued { blocked_by: vec![] },
        }
    }

    pub fn consolidate() -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Consolidate,
            task: "Memory consolidation".into(),
            conversation: None,
            priority: ActionKind::Consolidate.default_priority(),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            source_messages: vec![],
            phase: Phase::Queued { blocked_by: vec![] },
        }
    }

    pub fn outreach(
        task: String,
        conversation: Option<ConversationId>,
        authority: Authority,
    ) -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Outreach,
            task,
            conversation,
            priority: ActionKind::Outreach.default_priority(),
            authority,
            style_directive: None,
            cancelled_note: None,
            source_messages: vec![],
            phase: Phase::Queued { blocked_by: vec![] },
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    pub fn is_queued(&self) -> bool {
        matches!(self.phase, Phase::Queued { .. })
    }

    pub fn responded(&self) -> bool {
        match &self.phase {
            Phase::Running { progress, .. } => progress.read().map_or(false, |p| p.responded),
            Phase::Done { outcome } => outcome.responded,
            _ => false,
        }
    }
}
