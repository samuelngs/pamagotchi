use crate::state::{Delta, RelationshipStanding};
use crate::store::Thought;
use protocol::{ConversationId, InboundMessage, MemoryId};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{Notify, mpsc};
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
    Review,
    Research,
    Consolidate,
    Outreach,
    Ruminate,
}

impl ActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Respond => "respond",
            Self::Review => "review",
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
            Self::Review => 30,
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
    pub relationship_standing: RelationshipStanding,
    pub style_directive: Option<String>,
    pub cancelled_note: Option<String>,
    pub source_intent: Option<String>,
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
    cancellation: CancellationToken,
}

impl RunningState {
    pub fn new() -> Self {
        Self {
            responded: false,
            last_tool: String::new(),
            cancellation: CancellationToken::new(),
        }
    }

    pub fn request_cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

struct CancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancellationToken {
    fn new() -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_one();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            self.inner.notify.notified().await;
        }
    }
}

pub struct Outcome {
    pub responded: bool,
    pub attempted_send: bool,
    pub cancelled: bool,
    pub delta: Option<Delta>,
    pub pending_messages: Vec<InboundMessage>,
    pub review_messages: Vec<InboundMessage>,
    pub thoughts: Vec<Thought>,
    pub memories_formed: Vec<MemoryId>,
    pub recalled_memory_ids: Vec<MemoryId>,
    pub had_injections: bool,
    pub attempts: u32,
}

impl Default for Outcome {
    fn default() -> Self {
        Self {
            responded: false,
            attempted_send: false,
            cancelled: false,
            delta: None,
            pending_messages: vec![],
            review_messages: vec![],
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            had_injections: false,
            attempts: 0,
        }
    }
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
        relationship_standing: RelationshipStanding,
        style_directive: Option<String>,
    ) -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Respond,
            task: "Respond to message".into(),
            conversation: Some(conversation),
            priority: ActionKind::Respond.default_priority(),
            relationship_standing,
            style_directive,
            cancelled_note: None,
            source_intent: None,
            source_messages: messages,
            phase: Phase::Queued { blocked_by: vec![] },
        }
    }

    pub fn review(
        source_action: ActionId,
        messages: Vec<InboundMessage>,
        conversation: Option<ConversationId>,
        relationship_standing: RelationshipStanding,
    ) -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Review,
            task: format!("Review completed action {source_action}"),
            conversation,
            priority: ActionKind::Review.default_priority(),
            relationship_standing,
            style_directive: None,
            cancelled_note: Some(format!("Post-turn review for action {source_action}")),
            source_intent: None,
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
            relationship_standing: RelationshipStanding::Default,
            style_directive: None,
            cancelled_note: None,
            source_intent: None,
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
            relationship_standing: RelationshipStanding::Default,
            style_directive: None,
            cancelled_note: None,
            source_intent: None,
            source_messages: vec![],
            phase: Phase::Queued { blocked_by: vec![] },
        }
    }

    pub fn outreach(
        task: String,
        conversation: Option<ConversationId>,
        relationship_standing: RelationshipStanding,
    ) -> Self {
        Self::outreach_with_source_intent(task, conversation, relationship_standing, None)
    }

    pub fn outreach_with_source_intent(
        task: String,
        conversation: Option<ConversationId>,
        relationship_standing: RelationshipStanding,
        source_intent: Option<String>,
    ) -> Self {
        Self {
            id: ActionId::new(),
            kind: ActionKind::Outreach,
            task,
            conversation,
            priority: ActionKind::Outreach.default_priority(),
            relationship_standing,
            style_directive: None,
            cancelled_note: None,
            source_intent,
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
