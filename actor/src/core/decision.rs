use super::action::ActionId;
use protocol::InboundMessage;

#[derive(Debug)]
pub enum MindVerdict {
    Respond,
    Drop,
    Defer,
}

pub struct MindDecision {
    pub spawn: Vec<super::action::ActionRequest>,
    pub cancel: Vec<ActionId>,
    pub supplement: Vec<(ActionId, SupplementContext)>,
    pub inject: Vec<(ActionId, InboundMessage)>,
}

impl MindDecision {
    pub fn drop() -> Self {
        Self {
            spawn: vec![],
            cancel: vec![],
            supplement: vec![],
            inject: vec![],
        }
    }

    pub fn spawn_one(request: super::action::ActionRequest) -> Self {
        Self {
            spawn: vec![request],
            cancel: vec![],
            supplement: vec![],
            inject: vec![],
        }
    }

    pub fn cancel_and_spawn(cancel: Vec<ActionId>, request: super::action::ActionRequest) -> Self {
        Self {
            spawn: vec![request],
            cancel,
            supplement: vec![],
            inject: vec![],
        }
    }

    pub fn inject_one(action_id: ActionId, message: InboundMessage) -> Self {
        Self {
            spawn: vec![],
            cancel: vec![],
            supplement: vec![],
            inject: vec![(action_id, message)],
        }
    }
}

pub struct SupplementContext {
    pub messages: Vec<InboundMessage>,
    pub note: String,
}
