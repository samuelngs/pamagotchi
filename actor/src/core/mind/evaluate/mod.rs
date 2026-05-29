use super::super::action::{ActionId, RunningState};
use super::super::decision::{MindDecision, MindVerdict};
use super::super::event::{FiredIntent, WakeEvent};
use super::super::session::{self, SessionResult};
use super::super::tools::{SessionContext, SessionKind};
use super::{MAX_DEFER_COUNT, Mind};
use crate::state::{AdoptionRitualState, Authority};
use crate::store::ConversationSummary;
use inference::{Reasoning, RouteContext};
use protocol::{ConversationId, InboundMessage};

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{info, warn};

impl Mind {
    pub(super) async fn evaluate(&self, event: &WakeEvent) -> MindVerdict {
        let Some(evaluable) = EvaluableEvent::from_wake(event) else {
            return MindVerdict::Drop;
        };

        if let WakeEvent::Message(msg) = event
            && self.adoption_message_requires_response(msg)
        {
            return MindVerdict::Respond {
                style_directive: None,
            };
        }

        let event_desc = describe(evaluable);
        let eval_messages = self.evaluation_messages(evaluable, event_desc).await;

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
            typing: self.typing.clone(),
            metrics: self.metrics.clone(),
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

    pub(super) async fn build_decision(
        &self,
        verdict: MindVerdict,
        event: &WakeEvent,
    ) -> MindDecision {
        if matches!(self.resolve_authority(event), Authority::Blocked)
            && !matches!(event, WakeEvent::IntentFired(intent) if intent.chosen_human_approved)
        {
            tracing::info!("blocked person — dropping silently");
            return MindDecision::Drop;
        }

        match verdict {
            MindVerdict::Drop => MindDecision::Drop,
            MindVerdict::Defer { delay_secs } => self.defer_event(event, delay_secs),
            MindVerdict::Respond { style_directive } => {
                self.respond_to(event, style_directive).await
            }
        }
    }

    fn defer_event(&self, event: &WakeEvent, delay_secs: u64) -> MindDecision {
        match event {
            WakeEvent::Message(msg) => self.defer_message(msg, delay_secs),
            WakeEvent::IntentFired(intent) => self.defer_intent(intent, delay_secs),
            WakeEvent::ConsolidationDue => {
                MindDecision::DeferConsolidation(delay_secs.clamp(5, 300))
            }
            WakeEvent::IdleTick { .. }
            | WakeEvent::TypingUpdate { .. }
            | WakeEvent::MessageEdited { .. }
            | WakeEvent::MessageDeleted { .. }
            | WakeEvent::ActionCompleted { .. }
            | WakeEvent::Shutdown => MindDecision::Drop,
        }
    }

    pub(super) fn defer_message(&self, msg: &InboundMessage, delay_secs: u64) -> MindDecision {
        self.defer_message_with_reason(msg, delay_secs, None)
    }

    pub(super) fn defer_message_for_typing(
        &self,
        msg: &InboundMessage,
        delay_secs: u64,
    ) -> MindDecision {
        self.defer_message_with_reason(msg, delay_secs, Some("typing"))
    }

    fn defer_message_with_reason(
        &self,
        msg: &InboundMessage,
        delay_secs: u64,
        reason: Option<&str>,
    ) -> MindDecision {
        let count = defer_count(msg);
        if count >= MAX_DEFER_COUNT {
            info!(
                message_id = %msg.message_id,
                count,
                "message exceeded max defer count, dropping"
            );
            return MindDecision::Drop;
        }

        let mut deferred = msg.clone();
        set_defer_count(&mut deferred, count + 1);
        if let Some(reason) = reason {
            set_defer_reason(&mut deferred, reason);
        }
        MindDecision::DeferMessage(deferred, delay_secs.clamp(5, 300))
    }

    fn defer_intent(&self, intent: &FiredIntent, delay_secs: u64) -> MindDecision {
        if intent.defer_count >= MAX_DEFER_COUNT {
            info!(
                intent_id = %intent.id,
                count = intent.defer_count,
                "intent exceeded max defer count, dropping"
            );
            return MindDecision::Drop;
        }

        let mut deferred = intent.clone();
        deferred.defer_count += 1;
        MindDecision::DeferIntent(deferred, delay_secs.clamp(5, 300))
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

    fn adoption_message_requires_response(&self, msg: &InboundMessage) -> bool {
        let Some(person) = msg.person.as_ref() else {
            return false;
        };
        let actor = self.state.read_state();
        adoption_gate_forces_response(actor.adoption_state(person), &msg.content)
    }

    async fn evaluation_messages(
        &self,
        event: EvaluableEvent<'_>,
        event_desc: String,
    ) -> Vec<InboundMessage> {
        match event {
            EvaluableEvent::Message(msg) => vec![msg.clone()],
            EvaluableEvent::IntentFired(intent) => {
                vec![self.intent_evaluation_message(intent, event_desc).await]
            }
            _ => vec![control_event_message(event_desc)],
        }
    }

    async fn intent_evaluation_message(
        &self,
        intent: &FiredIntent,
        event_desc: String,
    ) -> InboundMessage {
        let summary = match intent.conversation.as_ref() {
            Some(conversation) => {
                self.store
                    .list_conversations()
                    .await
                    .ok()
                    .and_then(|conversations| {
                        conversations
                            .into_iter()
                            .find(|summary| &summary.id == conversation)
                    })
            }
            None => None,
        };
        intent_context_message(
            intent,
            event_desc,
            summary.as_ref(),
            chrono::Utc::now().timestamp(),
        )
    }
}

fn looks_safety_critical(content: &str) -> bool {
    let normalized = content.to_lowercase();
    [
        "suicide",
        "kill myself",
        "hurt myself",
        "self harm",
        "overdose",
        "emergency",
        "danger",
        "unsafe",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn adoption_gate_forces_response(state: Option<&AdoptionRitualState>, content: &str) -> bool {
    state.is_some_and(|state| {
        *state != AdoptionRitualState::AdoptionComplete && !looks_safety_critical(content)
    })
}

#[derive(Clone, Copy)]
enum EvaluableEvent<'a> {
    Message(&'a InboundMessage),
    IdleTick {
        elapsed_secs: f64,
    },
    ConsolidationDue,
    IntentFired(&'a FiredIntent),
    TypingUpdate {
        sender_external_id: &'a str,
        typing: bool,
    },
    MessageEdited {
        conversation: &'a ConversationId,
        message_id: &'a str,
    },
    MessageDeleted {
        conversation: &'a ConversationId,
        message_id: &'a str,
    },
}

impl<'a> EvaluableEvent<'a> {
    fn from_wake(event: &'a WakeEvent) -> Option<Self> {
        match event {
            WakeEvent::Message(msg) => Some(Self::Message(msg)),
            WakeEvent::IdleTick { elapsed_secs } => Some(Self::IdleTick {
                elapsed_secs: *elapsed_secs,
            }),
            WakeEvent::ConsolidationDue => Some(Self::ConsolidationDue),
            WakeEvent::IntentFired(intent) => Some(Self::IntentFired(intent)),
            WakeEvent::TypingUpdate {
                sender_external_id,
                typing,
                ..
            } => Some(Self::TypingUpdate {
                sender_external_id,
                typing: *typing,
            }),
            WakeEvent::MessageEdited {
                conversation,
                message_id,
                ..
            } => Some(Self::MessageEdited {
                conversation,
                message_id,
            }),
            WakeEvent::MessageDeleted {
                conversation,
                message_id,
                ..
            } => Some(Self::MessageDeleted {
                conversation,
                message_id,
            }),
            WakeEvent::ActionCompleted { .. } | WakeEvent::Shutdown => None,
        }
    }
}

fn defer_count(msg: &InboundMessage) -> u64 {
    msg.metadata
        .get("mind_defer_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn set_defer_count(msg: &mut InboundMessage, count: u64) {
    match &mut msg.metadata {
        serde_json::Value::Object(obj) => {
            obj.insert("mind_defer_count".into(), serde_json::json!(count));
        }
        serde_json::Value::Null => {
            msg.metadata = serde_json::json!({ "mind_defer_count": count });
        }
        other => {
            msg.metadata = serde_json::json!({
                "source_metadata": other.clone(),
                "mind_defer_count": count,
            });
        }
    }
}

fn set_defer_reason(msg: &mut InboundMessage, reason: &str) {
    match &mut msg.metadata {
        serde_json::Value::Object(obj) => {
            obj.insert("mind_defer_reason".into(), serde_json::json!(reason));
        }
        serde_json::Value::Null => {
            msg.metadata = serde_json::json!({ "mind_defer_reason": reason });
        }
        other => {
            msg.metadata = serde_json::json!({
                "source_metadata": other.clone(),
                "mind_defer_reason": reason,
            });
        }
    }
}

pub(super) fn defer_reason(msg: &InboundMessage) -> Option<&str> {
    msg.metadata
        .get("mind_defer_reason")
        .and_then(serde_json::Value::as_str)
}

fn describe(event: EvaluableEvent<'_>) -> String {
    match event {
        EvaluableEvent::Message(msg) => {
            format!(
                "New message in conversation {}:\n{}",
                msg.conversation.0,
                msg.display_content()
            )
        }
        EvaluableEvent::IdleTick { elapsed_secs } => {
            format!(
                "Idle tick. {:.0} seconds since last activity.",
                elapsed_secs
            )
        }
        EvaluableEvent::ConsolidationDue => "Periodic memory consolidation is due.".into(),
        EvaluableEvent::IntentFired(intent) => {
            let conv = intent
                .conversation
                .as_ref()
                .map_or("none".to_string(), |c| c.0.clone());
            format!(
                "Scheduled intent fired: {} (conversation: {})",
                intent.task, conv
            )
        }
        EvaluableEvent::TypingUpdate {
            sender_external_id,
            typing,
        } => {
            format!(
                "{} {} typing.",
                sender_external_id,
                if typing { "started" } else { "stopped" }
            )
        }
        EvaluableEvent::MessageEdited {
            message_id,
            conversation,
        } => {
            format!(
                "Message {message_id} in conversation {} was edited.",
                conversation.0
            )
        }
        EvaluableEvent::MessageDeleted {
            message_id,
            conversation,
        } => {
            format!(
                "Message {message_id} in conversation {} was deleted.",
                conversation.0
            )
        }
    }
}

fn control_event_message(event_desc: String) -> InboundMessage {
    InboundMessage {
        message_id: String::new(),
        gateway_id: String::new(),
        sender_external_id: String::new(),
        sender_display_name: None,
        reply_external_id: String::new(),
        conversation: ConversationId("mind".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
        content: event_desc,
        attachments: Vec::new(),
        timestamp: 0,
        metadata: serde_json::Value::Null,
    }
}

fn intent_context_message(
    intent: &FiredIntent,
    event_desc: String,
    summary: Option<&ConversationSummary>,
    now: i64,
) -> InboundMessage {
    let conversation = intent
        .conversation
        .clone()
        .or_else(|| summary.map(|summary| summary.id.clone()))
        .unwrap_or_else(|| ConversationId(format!("intent:{}", intent.id)));
    InboundMessage {
        message_id: format!("intent:{}", intent.id),
        gateway_id: summary
            .and_then(|summary| summary.gateway_id.clone())
            .unwrap_or_default(),
        sender_external_id: String::new(),
        sender_display_name: None,
        reply_external_id: String::new(),
        conversation,
        group: summary.and_then(|summary| summary.group.clone()),
        identity: summary.and_then(|summary| summary.identity.clone()),
        profile: summary.and_then(|summary| summary.profile.clone()),
        person: intent
            .person
            .clone()
            .or_else(|| summary.and_then(|summary| summary.person.clone())),
        content: event_desc,
        attachments: Vec::new(),
        timestamp: now,
        metadata: serde_json::json!({
            "event": "intent_fired",
            "intent_id": intent.id,
            "scheduled_at": intent.scheduled_at,
            "chosen_human_approved": intent.chosen_human_approved,
            "defer_count": intent.defer_count,
        }),
    }
}

#[cfg(test)]
mod tests;
