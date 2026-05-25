use super::action::{Action, ActionId};
use protocol::InboundMessage;

#[derive(Debug)]
pub enum MindVerdict {
    Respond { style_directive: Option<String> },
    Drop,
    Defer,
}

pub enum MindDecision {
    Drop,
    Spawn(Action),
    Inject(ActionId, InboundMessage),
    CancelAndSpawn(Vec<ActionId>, Action),
}
