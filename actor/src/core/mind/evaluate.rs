use super::super::action::{ActionId, ActionProgress};
use super::super::decision::{MindDecision, MindVerdict};
use super::super::event::WakeEvent;
use protocol::InboundMessage;
use super::super::session::{self, SessionResult};
use super::super::tools::{SessionContext, SessionKind};
use super::Mind;
use crate::personality::Authority;
use inference::RouteContext;

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::warn;

impl Mind {
    pub(super) async fn evaluate(&self, event: &WakeEvent) -> MindVerdict {
        let messages = match event {
            WakeEvent::Message(msg) => vec![msg.clone()],
            _ => vec![],
        };

        let event_desc = super::event::describe(event);

        let eval_messages = if messages.is_empty() {
            vec![InboundMessage {
                message_id: String::new(),
                gateway_id: String::new(),
                external_id: String::new(),
                conversation: protocol::ConversationId("mind".into()),
                group: None,
                person: None,
                content: event_desc,
                media: None,
                timestamp: 0,
                metadata: serde_json::Value::Null,
            }]
        } else {
            messages
        };

        let (_inject_tx, inject_rx) = mpsc::channel::<InboundMessage>(1);

        let endpoints = self.router.resolve_chain(&RouteContext::Mind);

        let ctx = SessionContext {
            action_id: ActionId::new(),
            kind: SessionKind::Mind,
            messages: eval_messages,
            conversation: None,
            authority: Authority::Default,
            state: self.state.clone(),
            store: self.store.clone(),
            endpoints,
            context: Some(self.gather_context(None)),
            inject_rx,
            progress: Arc::new(RwLock::new(ActionProgress::new())),
            max_turns: 5,
            gateway: self.gateway.clone(),
            session_start: std::time::Instant::now(),
        };

        match session::run_session(ctx).await {
            SessionResult::Mind(verdict) => verdict,
            SessionResult::Action(_) => {
                warn!("mind session returned action result, defaulting to respond");
                MindVerdict::Respond
            }
        }
    }

    pub(super) fn build_decision(&self, verdict: MindVerdict, event: &WakeEvent) -> MindDecision {
        if matches!(self.resolve_authority(event), Authority::Blocked) {
            tracing::info!("blocked person — dropping silently");
            return MindDecision::drop();
        }

        match verdict {
            MindVerdict::Drop | MindVerdict::Defer => MindDecision::drop(),
            MindVerdict::Respond => self.respond_to(event),
        }
    }

    pub(super) fn resolve_authority(&self, event: &WakeEvent) -> Authority {
        let person = match event {
            WakeEvent::Message(msg) => msg.person.as_ref(),
            WakeEvent::IntentFired(intent) => intent.person.as_ref(),
            _ => None,
        };
        let personality = self.state.read_personality();
        person
            .and_then(|p| personality.relationships.get(p))
            .map_or(Authority::Default, |r| r.authority.clone())
    }
}
