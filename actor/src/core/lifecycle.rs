use super::action::{Action, ActionId, ActionKind, Phase};
use super::decision::{MindDecision, MindVerdict};
use super::event::WakeEvent;
use protocol::{ConversationId, InboundMessage};

#[derive(Clone, Debug)]
pub enum ActorLifecycleEvent {
    MindStarted,
    MindStopped,
    MindEvaluated(MindLifecycleEvaluation),
    MindDecisionBuilt(MindLifecycleDecisionBuilt),
    ActionStarted(ActionLifecycleActionStarted),
    ActionCompleted(ActionLifecycleActionCompleted),
}

#[derive(Clone, Debug)]
pub struct MindLifecycleEvaluation {
    pub wake: MindLifecycleWake,
    pub verdict: MindLifecycleVerdict,
}

#[derive(Clone, Debug)]
pub struct MindLifecycleDecisionBuilt {
    pub wake: MindLifecycleWake,
    pub decision: MindLifecycleDecision,
}

#[derive(Clone, Debug)]
pub struct MindLifecycleWake {
    pub kind: String,
    pub conversation: Option<ConversationId>,
    pub source_message_keys: Vec<String>,
    pub intent_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum MindLifecycleVerdict {
    Respond { has_style_directive: bool },
    Drop,
    Defer { delay_secs: u64 },
}

#[derive(Clone, Debug)]
pub enum MindLifecycleDecision {
    Drop,
    Spawn {
        action_id: ActionId,
        action_kind: ActionKind,
    },
    Inject {
        action_id: ActionId,
    },
    DeferMessage {
        delay_secs: u64,
    },
    DeferIntent {
        intent_id: String,
        delay_secs: u64,
    },
    DeferConsolidation {
        delay_secs: u64,
    },
    CancelAndSpawn {
        cancelled_action_ids: Vec<ActionId>,
        action_id: ActionId,
        action_kind: ActionKind,
    },
}

#[derive(Clone, Debug)]
pub struct ActionLifecycleActionStarted {
    pub action_id: ActionId,
    pub kind: ActionKind,
    pub conversation: Option<ConversationId>,
    pub source_message_keys: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ActionLifecycleActionCompleted {
    pub action_id: ActionId,
    pub kind: ActionKind,
    pub conversation: Option<ConversationId>,
    pub source_message_keys: Vec<String>,
    pub responded: bool,
    pub attempted_send: bool,
    pub cancelled: bool,
    pub attempts: u32,
}

impl ActorLifecycleEvent {
    pub(crate) fn mind_evaluated(event: &WakeEvent, verdict: &MindVerdict) -> Self {
        Self::MindEvaluated(MindLifecycleEvaluation {
            wake: MindLifecycleWake::from_wake(event),
            verdict: MindLifecycleVerdict::from_verdict(verdict),
        })
    }

    pub(crate) fn mind_decision_built(event: &WakeEvent, decision: &MindDecision) -> Self {
        Self::MindDecisionBuilt(MindLifecycleDecisionBuilt {
            wake: MindLifecycleWake::from_wake(event),
            decision: MindLifecycleDecision::from_decision(decision),
        })
    }

    pub(crate) fn action_started(action: &Action) -> Self {
        Self::ActionStarted(ActionLifecycleActionStarted {
            action_id: action.id.clone(),
            kind: action.kind.clone(),
            conversation: action.conversation.clone(),
            source_message_keys: source_message_keys(&action.source_messages),
        })
    }

    pub(crate) fn action_completed(action: &Action) -> Option<Self> {
        let Phase::Done { outcome } = &action.phase else {
            return None;
        };
        Some(Self::ActionCompleted(ActionLifecycleActionCompleted {
            action_id: action.id.clone(),
            kind: action.kind.clone(),
            conversation: action.conversation.clone(),
            source_message_keys: source_message_keys(&action.source_messages),
            responded: outcome.responded,
            attempted_send: outcome.attempted_send,
            cancelled: outcome.cancelled,
            attempts: outcome.attempts,
        }))
    }
}

impl MindLifecycleWake {
    fn from_wake(event: &WakeEvent) -> Self {
        match event {
            WakeEvent::Message(message) => Self {
                kind: "message".into(),
                conversation: Some(message.conversation.clone()),
                source_message_keys: source_message_keys(std::slice::from_ref(message)),
                intent_id: None,
            },
            WakeEvent::IdleTick { .. } => Self::simple("idle_tick"),
            WakeEvent::ConsolidationDue => Self::simple("consolidation_due"),
            WakeEvent::IntentFired(intent) => Self {
                kind: "intent_fired".into(),
                conversation: intent.conversation.clone(),
                source_message_keys: vec![],
                intent_id: Some(intent.id.clone()),
            },
            WakeEvent::TypingUpdate { conversation, .. } => Self {
                kind: "typing_update".into(),
                conversation: Some(conversation.clone()),
                source_message_keys: vec![],
                intent_id: None,
            },
            WakeEvent::MessageEdited { conversation, .. } => Self {
                kind: "message_edited".into(),
                conversation: Some(conversation.clone()),
                source_message_keys: message_event_keys(event),
                intent_id: None,
            },
            WakeEvent::MessageDeleted { conversation, .. } => Self {
                kind: "message_deleted".into(),
                conversation: Some(conversation.clone()),
                source_message_keys: message_event_keys(event),
                intent_id: None,
            },
            WakeEvent::ActionCompleted { .. } => Self {
                kind: "action_completed".into(),
                conversation: None,
                source_message_keys: vec![],
                intent_id: None,
            },
            WakeEvent::Shutdown => Self::simple("shutdown"),
        }
    }

    fn simple(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            conversation: None,
            source_message_keys: vec![],
            intent_id: None,
        }
    }
}

impl MindLifecycleVerdict {
    fn from_verdict(verdict: &MindVerdict) -> Self {
        match verdict {
            MindVerdict::Respond { style_directive } => Self::Respond {
                has_style_directive: style_directive.is_some(),
            },
            MindVerdict::Drop => Self::Drop,
            MindVerdict::Defer { delay_secs } => Self::Defer {
                delay_secs: *delay_secs,
            },
        }
    }
}

impl MindLifecycleDecision {
    fn from_decision(decision: &MindDecision) -> Self {
        match decision {
            MindDecision::Drop => Self::Drop,
            MindDecision::Spawn(action) => Self::Spawn {
                action_id: action.id.clone(),
                action_kind: action.kind.clone(),
            },
            MindDecision::Inject(action_id, _) => Self::Inject {
                action_id: action_id.clone(),
            },
            MindDecision::DeferMessage(_, delay_secs) => Self::DeferMessage {
                delay_secs: *delay_secs,
            },
            MindDecision::DeferIntent(intent, delay_secs) => Self::DeferIntent {
                intent_id: intent.id.clone(),
                delay_secs: *delay_secs,
            },
            MindDecision::DeferConsolidation(delay_secs) => Self::DeferConsolidation {
                delay_secs: *delay_secs,
            },
            MindDecision::CancelAndSpawn(cancelled_action_ids, action) => Self::CancelAndSpawn {
                cancelled_action_ids: cancelled_action_ids.clone(),
                action_id: action.id.clone(),
                action_kind: action.kind.clone(),
            },
        }
    }
}

fn message_event_keys(event: &WakeEvent) -> Vec<String> {
    match event {
        WakeEvent::MessageEdited {
            gateway_id,
            message_id,
            ..
        }
        | WakeEvent::MessageDeleted {
            gateway_id,
            message_id,
            ..
        } => source_message_key(gateway_id, message_id)
            .into_iter()
            .collect(),
        _ => vec![],
    }
}

fn source_message_keys(messages: &[InboundMessage]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| source_message_key(&message.gateway_id, &message.message_id))
        .collect()
}

fn source_message_key(gateway_id: &str, message_id: &str) -> Option<String> {
    if gateway_id.is_empty() || message_id.is_empty() {
        None
    } else {
        Some(format!("{gateway_id}:{message_id}"))
    }
}
