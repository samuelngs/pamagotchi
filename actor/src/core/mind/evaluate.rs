use super::super::action::{ActionId, RunningState};
use super::super::decision::{MindDecision, MindVerdict};
use super::super::event::WakeEvent;
use super::super::session::{self, SessionResult};
use super::super::tools::{SessionContext, SessionKind};
use super::Mind;
use crate::state::Authority;
use inference::{Reasoning, RouteContext};
use protocol::InboundMessage;

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::warn;

impl Mind {
    pub(super) async fn evaluate(&self, event: &WakeEvent) -> MindVerdict {
        let messages = match event {
            WakeEvent::Message(msg) => vec![msg.clone()],
            _ => vec![],
        };

        let event_desc = describe(event);

        let eval_messages = if messages.is_empty() {
            vec![InboundMessage {
                message_id: String::new(),
                gateway_id: String::new(),
                external_id: String::new(),
                conversation: protocol::ConversationId("mind".into()),
                group: None,
                identity: None,
                profile: None,
                person: None,
                content: event_desc,
                attachments: Vec::new(),
                timestamp: 0,
                metadata: serde_json::Value::Null,
            }]
        } else {
            messages
        };

        let (_inject_tx, inject_rx) = mpsc::channel::<InboundMessage>(1);

        let endpoints = self.router.resolve_chain(&RouteContext::Mind);

        let concurrent_summaries: Vec<(String, String, String)> = self
            .registry
            .running()
            .iter()
            .map(|a| (a.id.0.clone(), format!("{:?}", a.kind), a.task.clone()))
            .collect();

        let ctx = SessionContext {
            action_id: ActionId::new(),
            kind: SessionKind::Mind,
            messages: eval_messages,
            conversation: None,
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries,
            state: self.state.clone(),
            store: self.store.clone(),
            media_store: self.media_store.clone(),
            router: self.router.clone(),
            endpoints,
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 5,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: self.gateway.clone(),
            session_start: std::time::Instant::now(),
        };

        match session::run_session(ctx).await {
            SessionResult::Mind(verdict) => verdict,
            SessionResult::Action(_) => {
                warn!("mind session returned action result, defaulting to respond");
                MindVerdict::Respond {
                    style_directive: None,
                }
            }
        }
    }

    pub(super) fn build_decision(&self, verdict: MindVerdict, event: &WakeEvent) -> MindDecision {
        if matches!(self.resolve_authority(event), Authority::Blocked) {
            tracing::info!("blocked person — dropping silently");
            return MindDecision::Drop;
        }

        match verdict {
            MindVerdict::Drop | MindVerdict::Defer => MindDecision::Drop,
            MindVerdict::Respond { style_directive } => self.respond_to(event, style_directive),
        }
    }

    pub(super) fn resolve_authority(&self, event: &WakeEvent) -> Authority {
        let person = match event {
            WakeEvent::Message(msg) => msg.person.as_ref(),
            WakeEvent::IntentFired(intent) => intent.person.as_ref(),
            _ => None,
        };
        let actor = self.state.read_state();
        person
            .and_then(|p| actor.bonds.get(p))
            .map_or(Authority::Default, |r| r.authority.clone())
    }
}

fn describe(event: &WakeEvent) -> String {
    match event {
        WakeEvent::Message(msg) => {
            format!(
                "New message in conversation {}:\n{}",
                msg.conversation.0,
                msg.display_content()
            )
        }
        WakeEvent::IdleTick { elapsed_secs } => {
            format!(
                "Idle tick. {:.0} seconds since last activity.",
                elapsed_secs
            )
        }
        WakeEvent::IntentFired(intent) => {
            let conv = intent
                .conversation
                .as_ref()
                .map_or("none".to_string(), |c| c.0.clone());
            format!(
                "Scheduled intent fired: {} (conversation: {})",
                intent.task, conv
            )
        }
        WakeEvent::ActionCompleted { action_id, outcome } => {
            let has_delta = outcome.delta.is_some();
            format!(
                "Action {} completed. responded={} personality_delta={}",
                action_id, outcome.responded, has_delta
            )
        }
        WakeEvent::TypingUpdate { person, typing, .. } => {
            format!(
                "{} {} typing.",
                person.0,
                if *typing { "started" } else { "stopped" }
            )
        }
        WakeEvent::Shutdown => unreachable!(),
    }
}
