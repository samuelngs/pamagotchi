mod activity;
mod completion;
mod evaluate;
mod injection;
mod maintenance;
mod message_events;
mod respond;
mod spawn;
mod triggered;
mod typing;
mod typing_state;

use super::action::{Action, ActionId, ActionKind, FollowUp};
use super::decision::MindDecision;
use super::event::{WakeEvent, claim_and_send_persisted_event};
use super::handle::StateHandle;
use super::ingest;
use super::metrics::ActorMetrics;
use super::registry::ActionRegistry;
use super::tools::{TYPING_ACTIVE_SECS, TypingState};
use crate::store::Store;
use activity::event_counts_as_activity;
use gateway::GatewayRouter;
use inference::InferenceRouter;
use injection::{mark_failed_injection_target, message_skips_injection_target};
#[cfg(test)]
use maintenance::{STALE_MEMORY_SECS, STALE_THOUGHT_SECS};
use media::MediaStore;
use protocol::InboundMessage;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use triggered::triggered_intent_satisfied_by_inbound_response;
use typing::{pending_typing_message_error, typing_deferred_message_matches};

pub(super) const MAX_DEFER_COUNT: u64 = 3;

pub struct Mind {
    pub(super) event_rx: mpsc::Receiver<WakeEvent>,
    pub(super) event_tx: mpsc::Sender<WakeEvent>,
    pub(super) registry: ActionRegistry,
    pub(super) state: StateHandle,
    pub(super) store: Arc<dyn Store>,
    pub(super) media_store: Option<Arc<MediaStore>>,
    pub(super) router: Arc<InferenceRouter>,
    pub(super) gateway: Arc<GatewayRouter>,
    pub(super) max_turns: usize,
    pub(super) max_action_attempts: usize,
    pub(super) escalate_after: usize,
    pub(super) metrics: Arc<ActorMetrics>,
    pub(super) reviewed_actions: HashSet<ActionId>,
    pub(super) typing: TypingState,
    pub(super) last_activity_at: Option<Instant>,
}

impl Mind {
    pub fn new(
        event_rx: mpsc::Receiver<WakeEvent>,
        event_tx: mpsc::Sender<WakeEvent>,
        state: StateHandle,
        store: Arc<dyn Store>,
        media_store: Option<Arc<MediaStore>>,
        router: Arc<InferenceRouter>,
        gateway: Arc<GatewayRouter>,
        max_concurrency: usize,
        max_turns: usize,
        max_action_attempts: usize,
        escalate_after: usize,
        metrics: Arc<ActorMetrics>,
    ) -> Self {
        Self {
            event_rx,
            event_tx,
            registry: ActionRegistry::new(max_concurrency),
            state,
            store,
            media_store,
            router,
            gateway,
            max_turns,
            max_action_attempts,
            escalate_after,
            metrics,
            reviewed_actions: HashSet::new(),
            typing: Arc::new(RwLock::new(Default::default())),
            last_activity_at: None,
        }
    }

    pub async fn run(mut self) {
        info!("mind started");
        loop {
            match self.event_rx.recv().await {
                Some(WakeEvent::Shutdown) => {
                    self.metrics.record_event_received();
                    self.metrics
                        .set_event_queue_depth(self.event_rx.len() as u64);
                    self.shutdown().await;
                    break;
                }
                Some(WakeEvent::ActionCompleted { action_id, outcome }) => {
                    self.metrics.record_event_received();
                    self.metrics
                        .set_event_queue_depth(self.event_rx.len() as u64);
                    self.record_activity();
                    self.handle_completed(&action_id, outcome).await;
                }
                Some(mut event) => {
                    self.metrics.record_event_received();
                    self.metrics
                        .set_event_queue_depth(self.event_rx.len() as u64);
                    if let WakeEvent::TypingUpdate {
                        conversation,
                        gateway_id,
                        sender_external_id,
                        typing,
                    } = &event
                    {
                        self.record_activity();
                        self.update_typing(conversation, gateway_id, sender_external_id, *typing);
                        if !typing {
                            self.flush_deferred_typing_messages(
                                conversation,
                                gateway_id,
                                sender_external_id,
                            )
                            .await;
                        }
                        continue;
                    }
                    if let WakeEvent::MessageEdited {
                        conversation,
                        gateway_id,
                        message_id,
                        content,
                        edited_at,
                    } = &event
                    {
                        self.record_activity();
                        self.apply_message_edit(
                            conversation,
                            gateway_id,
                            message_id,
                            content,
                            *edited_at,
                        )
                        .await;
                        continue;
                    }
                    if let WakeEvent::MessageDeleted {
                        conversation,
                        gateway_id,
                        message_id,
                        deleted_at,
                    } = &event
                    {
                        self.record_activity();
                        self.apply_message_delete(
                            conversation,
                            gateway_id,
                            message_id,
                            *deleted_at,
                        )
                        .await;
                        continue;
                    }
                    if let WakeEvent::IdleTick { elapsed_secs } = &event {
                        let pruned = self.prune_stale_typing(chrono::Utc::now().timestamp());
                        if pruned > 0 {
                            debug!(count = pruned, "pruned stale typing state");
                        }
                        if !self.idle_tick_is_due(*elapsed_secs) {
                            debug!(
                                elapsed_secs,
                                "skipping idle tick because recent activity was observed"
                            );
                            continue;
                        }
                        self.state.tick_idle(*elapsed_secs).await;
                    }
                    if matches!(event, WakeEvent::ConsolidationDue) {
                        self.prune_stale_context(chrono::Utc::now().timestamp())
                            .await;
                        let decision = self.respond_to(&event, None).await;
                        self.execute_decision(decision).await;
                        self.registry.gc();
                        debug!(
                            recent_completed = self.registry.recent_completed().len(),
                            "action registry garbage collection complete"
                        );
                        continue;
                    }
                    if let WakeEvent::Message(ref mut msg) = event {
                        self.record_activity();
                        ingest::resolve_person(&self.state, &self.store, msg).await;
                        ingest::observe_inbound(&self.state, msg).await;
                    } else if event_counts_as_activity(&event) {
                        self.record_activity();
                    }
                    if let WakeEvent::Message(ref msg) = event {
                        if let Some(target) = self.registry.unreplied_in(&msg.conversation) {
                            let target_id = target.id.clone();
                            if message_skips_injection_target(msg, &target_id) {
                                debug!(
                                    %target_id,
                                    message_id = %msg.message_id,
                                    "skipping previously failed injection target"
                                );
                            } else if let Some(sender) = self.registry.injection_sender(&target_id)
                            {
                                match sender.send(msg.clone()).await {
                                    Ok(()) => {
                                        self.metrics.record_injection(true);
                                        info!(%target_id, "injected message into running action");
                                        continue;
                                    }
                                    Err(err) => {
                                        self.metrics.record_injection(false);
                                        warn!(%target_id, "running action injection channel closed");
                                        let msg = mark_failed_injection_target(err.0, &target_id);
                                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    let verdict = self.evaluate(&event).await;
                    let decision = self.build_decision(verdict, &event).await;
                    self.retire_dropped_fired_intent(&event, &decision).await;
                    self.execute_decision(decision).await;
                    self.registry.gc();
                    debug!(
                        recent_completed = self.registry.recent_completed().len(),
                        "action registry garbage collection complete"
                    );
                }
                None => {
                    info!("event channel closed, shutting down");
                    self.shutdown().await;
                    break;
                }
            }
        }
        info!("mind stopped");
    }

    async fn shutdown(&mut self) {
        info!("mind shutting down, cancelling all actions");
        let running: Vec<ActionId> = self
            .registry
            .running()
            .iter()
            .map(|a| a.id.clone())
            .collect();
        for id in &running {
            self.registry.cancel(id);
            self.metrics.record_action_cancelled();
            self.store
                .finish_action_run(
                    &id.0,
                    chrono::Utc::now().timestamp(),
                    "cancelled",
                    false,
                    0,
                    vec![],
                    vec![],
                )
                .await
                .ok();
        }
        self.refresh_registry_metrics();
    }

    pub(super) fn schedule_action(&mut self, action: Action) -> ActionId {
        let id = self.registry.schedule(action);
        self.metrics.record_action_spawned();
        self.refresh_registry_metrics();
        id
    }

    pub(super) fn refresh_registry_metrics(&self) {
        self.metrics.observe_registry(
            self.registry.queued_len() as u64,
            self.registry.running_len() as u64,
            self.registry.retained_completed_len() as u64,
        );
    }
}

#[cfg(test)]
mod tests;
