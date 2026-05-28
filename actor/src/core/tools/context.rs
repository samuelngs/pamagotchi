use super::super::action::{ActionId, ActionKind, RunningState};
use super::super::decision::MindVerdict;
use super::super::handle::StateHandle;
use super::super::metrics::ActorMetrics;
use crate::state::{Authority, Delta};
use crate::store::{Store, Thought};
use gateway::GatewayRouter;
use inference::Reasoning;
use media::MediaStore;
use protocol::{ConversationId, InboundMessage, MemoryId};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

pub const TYPING_ACTIVE_SECS: i64 = 10;
pub type TypingStateKey = (ConversationId, String, String);
pub type TypingState = Arc<RwLock<HashMap<TypingStateKey, i64>>>;

pub struct SessionContext {
    pub action_id: ActionId,
    pub kind: SessionKind,
    pub messages: Vec<InboundMessage>,
    pub conversation: Option<ConversationId>,
    pub authority: Authority,
    pub style_directive: Option<String>,
    pub cancelled_note: Option<String>,
    pub concurrent_summaries: Vec<(String, String, String)>,
    pub state: StateHandle,
    pub store: Arc<dyn Store>,
    pub media_store: Option<Arc<MediaStore>>,
    pub router: Arc<inference::InferenceRouter>,
    pub endpoints: Vec<inference::ResolvedInference>,
    pub reasoning: Reasoning,
    pub inject_rx: mpsc::Receiver<InboundMessage>,
    pub progress: Arc<RwLock<RunningState>>,
    pub max_turns: usize,
    pub max_action_attempts: usize,
    pub escalate_after: usize,
    pub gateway: Arc<GatewayRouter>,
    pub typing: TypingState,
    pub metrics: Arc<ActorMetrics>,
    pub session_start: std::time::Instant,
}

pub enum SessionKind {
    Mind,
    Action(ActionKind),
}

pub struct SessionState {
    pub responded: bool,
    pub attempted_send: bool,
    pub composing_released: bool,
    pub delta: Delta,
    pub thoughts: Vec<Thought>,
    pub memories_formed: Vec<MemoryId>,
    pub recalled_memory_ids: Vec<MemoryId>,
    pub injected_messages: Vec<InboundMessage>,
    pub presented_injected_messages: Vec<InboundMessage>,
    pub presented_read_messages: Vec<InboundMessage>,
    pub pending_injected_messages: Vec<InboundMessage>,
    pub source_message_keys: HashSet<String>,
    pub queued_injected_message_keys: HashSet<String>,
    pub presented_injected_message_keys: HashSet<String>,
    pub applied_review_keys: HashSet<String>,
    pub presented_injection_count: usize,
}

pub enum ToolOutcome {
    Result(String),
    Decision(MindVerdict),
}
