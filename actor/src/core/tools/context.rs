use super::super::action::{ActionContext, ActionId, ActionKind, ActionProgress};
use super::super::decision::MindVerdict;
use super::super::state::StateHandle;
use crate::personality::{Authority, PersonalityDelta};
use crate::store::{Store, Thought};
use gateway::GatewayRouter;
use protocol::{ConversationId, InboundMessage, MemoryId};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

pub struct SessionContext {
    pub action_id: ActionId,
    pub kind: SessionKind,
    pub messages: Vec<InboundMessage>,
    pub conversation: Option<ConversationId>,
    pub authority: Authority,
    pub state: StateHandle,
    pub store: Arc<dyn Store>,
    pub endpoints: Vec<inference::ResolvedInference>,
    pub context: Option<ActionContext>,
    pub inject_rx: mpsc::Receiver<InboundMessage>,
    pub progress: Arc<RwLock<ActionProgress>>,
    pub max_turns: usize,
    pub gateway: Arc<GatewayRouter>,
    pub session_start: std::time::Instant,
}

pub enum SessionKind {
    Mind,
    Action(ActionKind),
}

#[allow(dead_code)]
pub struct SessionState {
    pub responded: bool,
    pub composing_released: bool,
    pub delta: PersonalityDelta,
    pub thoughts: Vec<Thought>,
    pub memories_formed: Vec<MemoryId>,
    pub injected_messages: Vec<InboundMessage>,
}

pub enum ToolOutcome {
    Result(String),
    Decision(MindVerdict),
}
