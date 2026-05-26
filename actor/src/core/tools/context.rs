use super::super::action::{ActionId, ActionKind, RunningState};
use super::super::decision::MindVerdict;
use super::super::handle::StateHandle;
use crate::state::{Authority, Delta};
use crate::store::{Store, Thought};
use gateway::GatewayRouter;
use inference::Reasoning;
use media::MediaStore;
use protocol::{ConversationId, InboundMessage, MemoryId};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

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
    pub session_start: std::time::Instant,
}

pub enum SessionKind {
    Mind,
    Action(ActionKind),
}

pub struct SessionState {
    pub responded: bool,
    pub composing_released: bool,
    pub delta: Delta,
    pub thoughts: Vec<Thought>,
    pub memories_formed: Vec<MemoryId>,
    pub injected_messages: Vec<InboundMessage>,
}

pub enum ToolOutcome {
    Result(String),
    Decision(MindVerdict),
}
