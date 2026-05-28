use super::super::action::ActionId;
use super::super::decision::MindDecision;
use super::super::event::{WakeEvent, claim_and_send_persisted_event};
use super::super::session::{self, SessionResult};
use super::super::tools::{SessionContext, SessionKind};
use super::{Mind, mark_failed_injection_target};
use crate::store::{ActionMessageRecord, ActionRunRecord, EventInboxRecord};
use inference::{Reasoning, RouteContext};
use tracing::{info, warn};

impl Mind {
    pub(super) async fn execute_decision(&mut self, decision: MindDecision) {
        match decision {
            MindDecision::Drop => {
                self.metrics.record_event_dropped();
            }
            MindDecision::Spawn(action) => {
                let id = self.schedule_action(action);
                self.launch_action(&id).await;
            }
            MindDecision::Inject(id, msg) => {
                if let Some(sender) = self.registry.injection_sender(&id) {
                    match sender.send(msg).await {
                        Ok(()) => self.metrics.record_injection(true),
                        Err(err) => {
                            self.metrics.record_injection(false);
                            warn!(%id, "failed to inject message");
                            let msg = mark_failed_injection_target(err.0, &id);
                            self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                        }
                    }
                } else {
                    self.metrics.record_injection(false);
                    warn!(%id, "no running action injection channel");
                    let msg = mark_failed_injection_target(msg, &id);
                    self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                }
            }
            MindDecision::DeferMessage(msg, delay_secs) => {
                self.metrics.record_event_deferred();
                let event_tx = self.event_tx.clone();
                let store = self.store.clone();
                let event_id = self
                    .enqueue_deferred_event(
                        "message",
                        serde_json::to_value(&msg),
                        delay_secs,
                        deferred_message_dedupe_key(&msg),
                    )
                    .await;
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    if let Some(event_id) = event_id {
                        claim_and_send_persisted_event(
                            &event_tx,
                            store.as_ref(),
                            &event_id,
                            chrono::Utc::now().timestamp(),
                            WakeEvent::Message(msg),
                            "deferred message sleeper",
                        )
                        .await;
                    } else {
                        event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                });
            }
            MindDecision::DeferIntent(intent, delay_secs) => {
                self.metrics.record_event_deferred();
                let event_tx = self.event_tx.clone();
                let store = self.store.clone();
                let event_id = self
                    .enqueue_deferred_event(
                        "intent_fired",
                        serde_json::to_value(&intent),
                        delay_secs,
                        Some(format!(
                            "intent-fired:{}:{}",
                            &intent.id, intent.defer_count
                        )),
                    )
                    .await;
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    if let Some(event_id) = event_id {
                        claim_and_send_persisted_event(
                            &event_tx,
                            store.as_ref(),
                            &event_id,
                            chrono::Utc::now().timestamp(),
                            WakeEvent::IntentFired(intent),
                            "deferred intent sleeper",
                        )
                        .await;
                    } else {
                        event_tx.send(WakeEvent::IntentFired(intent)).await.ok();
                    }
                });
            }
            MindDecision::DeferConsolidation(delay_secs) => {
                self.metrics.record_event_deferred();
                let event_tx = self.event_tx.clone();
                let store = self.store.clone();
                let event_id = self
                    .enqueue_deferred_event(
                        "consolidation_due",
                        Ok(serde_json::json!({})),
                        delay_secs,
                        Some("consolidation-due".into()),
                    )
                    .await;
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    if let Some(event_id) = event_id {
                        claim_and_send_persisted_event(
                            &event_tx,
                            store.as_ref(),
                            &event_id,
                            chrono::Utc::now().timestamp(),
                            WakeEvent::ConsolidationDue,
                            "deferred consolidation sleeper",
                        )
                        .await;
                    } else {
                        event_tx.send(WakeEvent::ConsolidationDue).await.ok();
                    }
                });
            }
            MindDecision::CancelAndSpawn(cancel_ids, action) => {
                for cid in &cancel_ids {
                    if self.registry.cancel(cid) {
                        self.store
                            .finish_action_run(
                                &cid.0,
                                chrono::Utc::now().timestamp(),
                                "cancelled",
                                false,
                                0,
                                vec![],
                                vec![],
                            )
                            .await
                            .ok();
                        self.metrics.record_action_cancelled();
                        info!(%cid, "cancelled action");
                    }
                }
                self.refresh_registry_metrics();
                let id = self.schedule_action(action);
                self.launch_action(&id).await;
            }
        }
    }

    async fn enqueue_deferred_event(
        &self,
        kind: &str,
        payload: Result<serde_json::Value, serde_json::Error>,
        delay_secs: u64,
        dedupe_key: Option<String>,
    ) -> Option<String> {
        let payload = match payload {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, kind, "failed to serialize deferred event");
                return None;
            }
        };
        let now = chrono::Utc::now().timestamp();
        let event_id = format!("event-{}", nanoid::nanoid!());
        let record = EventInboxRecord {
            id: event_id.clone(),
            kind: kind.into(),
            payload,
            status: "pending".into(),
            due_at: now + delay_secs as i64,
            attempts: 0,
            dedupe_key,
            created_at: now,
            updated_at: now,
            fired_at: None,
            last_error: None,
        };
        match self.store.enqueue_event(&record).await {
            Ok(()) => Some(event_id),
            Err(e) => {
                warn!(%e, kind, "failed to persist deferred event");
                None
            }
        }
    }

    pub(super) async fn launch_action(&mut self, id: &ActionId) {
        let launch = match self.registry.launch(id) {
            Some(l) => l,
            None => return,
        };
        self.refresh_registry_metrics();

        let action = match self.registry.get(id) {
            Some(a) => a,
            None => return,
        };

        let kind = action.kind.clone();
        let task_desc = action.task.clone();
        let conversation = action.conversation.clone();
        let authority = action.authority.clone();
        let style_directive = action.style_directive.clone();
        let cancelled_note = action.cancelled_note.clone();
        let action_id = id.clone();

        let concurrent_summaries: Vec<(String, String, String)> = self
            .registry
            .running()
            .iter()
            .filter(|a| a.id != action_id)
            .map(|a| (a.id.0.clone(), format!("{:?}", a.kind), a.task.clone()))
            .collect();

        let event_tx = self.event_tx.clone();
        let state_handle = self.state.clone();
        let store = self.store.clone();
        let media_store = self.media_store.clone();
        let router = self.router.clone();
        let gateway = self.gateway.clone();
        let typing = self.typing.clone();
        let metrics = self.metrics.clone();
        let max_turns = self.max_turns;

        let reasoning = Reasoning::Standard;
        let endpoints = self.router.resolve_chain(&RouteContext::Action(reasoning));
        let max_action_attempts = self.max_action_attempts;
        let escalate_after = self.escalate_after;

        let handle = tokio::spawn(async move {
            info!(%action_id, kind = ?kind, task = %task_desc, "action started");
            let started_at = chrono::Utc::now().timestamp();
            let messages = launch.messages;

            let run = ActionRunRecord {
                action_id: action_id.0.clone(),
                kind: kind.as_str().to_string(),
                task: task_desc.clone(),
                conversation: conversation.clone(),
                started_at,
                ended_at: None,
                status: "running".into(),
                responded: false,
                attempts: 0,
            };
            if let Err(e) = store.start_action_run(&run).await {
                warn!(%action_id, %e, "failed to persist action run start");
            }
            for msg in &messages {
                let record = ActionMessageRecord {
                    action_id: action_id.0.clone(),
                    role: "user".into(),
                    conversation: Some(msg.conversation.clone()),
                    source_gateway_id: Some(msg.gateway_id.clone()),
                    source_message_id: Some(msg.message_id.clone()),
                    sender_external_id: Some(msg.sender_external_id.clone()),
                    reply_external_id: Some(msg.reply_external_id.clone()),
                    content: Some(msg.display_content()),
                    created_at: msg.timestamp,
                };
                if let Err(e) = store.append_action_message(&record).await {
                    warn!(%action_id, %e, "failed to persist action source message link");
                }
            }

            let ctx = SessionContext {
                action_id: action_id.clone(),
                kind: SessionKind::Action(kind),
                messages,
                conversation,
                authority,
                style_directive,
                cancelled_note,
                concurrent_summaries,
                state: state_handle,
                store: store.clone(),
                media_store,
                router,
                endpoints,
                reasoning,
                inject_rx: launch.inject_rx,
                progress: launch.progress,
                max_turns,
                max_action_attempts,
                escalate_after,
                gateway,
                typing,
                metrics,
                session_start: std::time::Instant::now(),
            };

            let outcome = match session::run_session(ctx).await {
                SessionResult::Action(outcome) => outcome,
                SessionResult::Mind(_) => {
                    warn!(%action_id, "action session returned mind result");
                    super::super::action::Outcome::default()
                }
            };

            if outcome.delta.is_some() {
                info!(%action_id, "action produced personality delta");
            }
            let status = if outcome.cancelled {
                "cancelled"
            } else {
                "completed"
            };
            if let Err(e) = store
                .finish_action_run(
                    &action_id.0,
                    chrono::Utc::now().timestamp(),
                    status,
                    outcome.responded,
                    outcome.attempts,
                    outcome.memories_formed.clone(),
                    outcome.recalled_memory_ids.clone(),
                )
                .await
            {
                warn!(%action_id, %e, "failed to persist action run finish");
            }

            event_tx
                .send(WakeEvent::ActionCompleted {
                    action_id: action_id.clone(),
                    outcome,
                })
                .await
                .ok();

            info!(%action_id, "action completed");
        });

        self.registry.set_handle(id, handle);
    }
}

fn deferred_message_dedupe_key(msg: &protocol::InboundMessage) -> Option<String> {
    if msg.gateway_id.is_empty() || msg.message_id.is_empty() {
        return None;
    }
    let count = msg
        .metadata
        .get("mind_defer_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Some(format!(
        "message:{}:{}:{count}",
        msg.gateway_id, msg.message_id
    ))
}
