use super::action::{Action, ActionId};
use super::event::FiredIntent;
use protocol::InboundMessage;

#[derive(Debug)]
pub enum MindVerdict {
    Respond { style_directive: Option<String> },
    Drop,
    Defer { delay_secs: u64 },
}

pub enum MindDecision {
    Drop,
    Spawn(Action),
    Inject(ActionId, InboundMessage),
    DeferMessage(InboundMessage, u64),
    DeferIntent(FiredIntent, u64),
    DeferConsolidation(u64),
    CancelAndSpawn(Vec<ActionId>, Action),
}
