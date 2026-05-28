mod evaluate;
mod respond;
mod spawn;

use super::action::{Action, ActionId, ActionKind, FollowUp};
use super::decision::MindDecision;
use super::event::{WakeEvent, claim_and_send_persisted_event};
use super::handle::StateHandle;
use super::ingest;
use super::metrics::ActorMetrics;
use super::registry::ActionRegistry;
use super::tools::{TYPING_ACTIVE_SECS, TypingState};
use crate::store::{IntentRecord, Store};
use gateway::GatewayRouter;
use inference::InferenceRouter;
use media::MediaStore;
use protocol::InboundMessage;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub(super) const MAX_DEFER_COUNT: u64 = 3;
const FAILED_INJECTION_ACTION_IDS: &str = "failed_injection_action_ids";
const TYPING_FLUSH_SCAN_LIMIT: usize = 64;
const STALE_THOUGHT_SECS: i64 = 30 * 24 * 60 * 60;
const STALE_THOUGHT_MAX_IMPORTANCE: f32 = 0.3;
const STALE_THOUGHT_MAX_CONFIDENCE: f32 = 0.3;
const STALE_THOUGHT_PRUNE_LIMIT: usize = 1000;
const STALE_MEMORY_SECS: i64 = 90 * 24 * 60 * 60;
const STALE_MEMORY_MAX_IMPORTANCE: f32 = 0.3;
const STALE_MEMORY_MAX_CONFIDENCE: f32 = 0.5;
const STALE_MEMORY_MAX_SENSITIVITY: f32 = 0.4;
const STALE_MEMORY_PRUNE_LIMIT: usize = 500;

fn typing_deferred_message_matches(
    msg: &InboundMessage,
    conversation: &protocol::ConversationId,
    gateway_id: &str,
    sender_external_id: &str,
) -> bool {
    evaluate::defer_reason(msg) == Some("typing")
        && &msg.conversation == conversation
        && msg.gateway_id.as_str() == gateway_id
        && msg.sender_external_id.as_str() == sender_external_id
}

fn event_counts_as_activity(event: &WakeEvent) -> bool {
    matches!(
        event,
        WakeEvent::ActionCompleted { .. }
            | WakeEvent::IntentFired(_)
            | WakeEvent::TypingUpdate { .. }
            | WakeEvent::MessageEdited { .. }
            | WakeEvent::MessageDeleted { .. }
    )
}

fn pending_typing_message_error(prior: Option<&str>, error: String) -> String {
    match prior {
        Some(prior) if !prior.is_empty() => format!("{prior}; {error}"),
        _ => error,
    }
}

pub(super) fn message_skips_injection_target(msg: &InboundMessage, target_id: &ActionId) -> bool {
    msg.metadata
        .get(FAILED_INJECTION_ACTION_IDS)
        .and_then(serde_json::Value::as_array)
        .is_some_and(|ids| {
            ids.iter()
                .any(|id| id.as_str() == Some(target_id.0.as_str()))
        })
}

pub(super) fn mark_failed_injection_target(
    mut msg: InboundMessage,
    target_id: &ActionId,
) -> InboundMessage {
    let mut obj = match msg.metadata {
        serde_json::Value::Object(obj) => obj,
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            let mut obj = serde_json::Map::new();
            obj.insert("source_metadata".into(), other);
            obj
        }
    };

    let mut ids = obj
        .remove(FAILED_INJECTION_ACTION_IDS)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    if !ids
        .iter()
        .any(|id| id.as_str() == Some(target_id.0.as_str()))
    {
        ids.push(serde_json::json!(target_id.0));
    }
    obj.insert(
        FAILED_INJECTION_ACTION_IDS.into(),
        serde_json::Value::Array(ids),
    );
    msg.metadata = serde_json::Value::Object(obj);
    msg
}

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
    reviewed_actions: HashSet<ActionId>,
    typing: TypingState,
    last_activity_at: Option<Instant>,
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

    async fn retire_dropped_fired_intent(&self, event: &WakeEvent, decision: &MindDecision) {
        if !matches!(decision, MindDecision::Drop) {
            return;
        }
        let WakeEvent::IntentFired(intent) = event else {
            return;
        };

        let stored = match self.store.get_intent(&intent.id).await {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                warn!(
                    intent_id = %intent.id,
                    "dropped fired intent was missing from store"
                );
                return;
            }
            Err(e) => {
                warn!(
                    %e,
                    intent_id = %intent.id,
                    "failed to load dropped fired intent"
                );
                return;
            }
        };
        if stored.status != "fired" {
            return;
        }

        let now = chrono::Utc::now().timestamp();
        match self.store.complete_intent(&intent.id, now).await {
            Ok(true) => info!(
                intent_id = %intent.id,
                "retired dropped fired intent"
            ),
            Ok(false) => {}
            Err(e) => warn!(
                %e,
                intent_id = %intent.id,
                "failed to retire dropped fired intent"
            ),
        }
    }

    async fn handle_completed(&mut self, action_id: &ActionId, outcome: super::action::Outcome) {
        if !self.registry.complete(action_id, outcome) {
            debug!(%action_id, "ignoring completion for action that is no longer running");
            return;
        }
        if let Some(action) = self.registry.get(action_id) {
            let failed = action.kind.expects_response() && !action.responded();
            self.metrics.record_action_completed(failed);
        }
        self.refresh_registry_metrics();

        if let Some(action) = self.registry.get(action_id) {
            if let super::action::Phase::Done { outcome } = &action.phase {
                if let Some(ref delta) = outcome.delta {
                    self.state.send_delta(delta.clone()).await;
                    info!(%action_id, "forwarded personality delta");
                }
            }
        }

        self.complete_successful_outreach_source_intent(action_id)
            .await;

        if let Some(review) = self.build_post_turn_review(action_id) {
            let review_action_id = review.id.clone();
            match self
                .store
                .mark_review_scheduled(
                    &action_id.0,
                    &review_action_id.0,
                    chrono::Utc::now().timestamp(),
                )
                .await
            {
                Ok(true) => {
                    self.reviewed_actions.insert(action_id.clone());
                    let review_id = self.schedule_action(review);
                    info!(%action_id, %review_id, "scheduled post-turn review");
                }
                Ok(false) => {
                    self.reviewed_actions.insert(action_id.clone());
                    info!(%action_id, "post-turn review already scheduled");
                }
                Err(e) => {
                    warn!(%action_id, %e, "failed to persist review watermark; skipping review scheduling");
                }
            }
        }

        self.retire_handled_triggered_intents(action_id).await;

        let follow_ups = self.registry.follow_ups(action_id);
        for fu in follow_ups {
            match fu {
                FollowUp::Requeue(msgs) => {
                    for msg in msgs {
                        info!(%action_id, "re-queuing source message after failed respond");
                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                }
                FollowUp::ReemitPending(msgs) => {
                    for msg in msgs {
                        info!(%action_id, "re-emitting pending message");
                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                }
            }
        }

        let ready = self.registry.ready();
        for id in ready {
            self.launch_action(&id).await;
        }

        self.registry.gc();
        self.refresh_registry_metrics();
        debug!(
            recent_completed = self.registry.recent_completed().len(),
            "action registry garbage collection complete"
        );
    }

    async fn complete_successful_outreach_source_intent(&self, action_id: &ActionId) {
        let Some(action) = self.registry.get(action_id) else {
            return;
        };
        if !matches!(action.kind, ActionKind::Outreach) {
            return;
        }
        let super::action::Phase::Done { outcome } = &action.phase else {
            return;
        };
        if !outcome.responded {
            return;
        }
        let Some(intent_id) = action.source_intent.as_deref() else {
            return;
        };

        let intent = match self.store.get_intent(intent_id).await {
            Ok(Some(intent)) => intent,
            Ok(None) => {
                warn!(
                    %action_id,
                    intent_id,
                    "outreach source intent was missing after successful action"
                );
                return;
            }
            Err(e) => {
                warn!(
                    %e,
                    %action_id,
                    intent_id,
                    "failed to load outreach source intent after successful action"
                );
                return;
            }
        };
        if intent.status != "fired" {
            return;
        }

        let now = chrono::Utc::now().timestamp();
        match self.store.complete_intent(intent_id, now).await {
            Ok(true) => info!(
                %action_id,
                intent_id,
                "marked successful outreach source intent completed"
            ),
            Ok(false) => {}
            Err(e) => warn!(
                %e,
                %action_id,
                intent_id,
                "failed to mark successful outreach source intent completed"
            ),
        }
    }

    fn build_post_turn_review(&mut self, action_id: &ActionId) -> Option<Action> {
        let action = self.registry.get(action_id)?;
        let super::action::Phase::Done { outcome } = &action.phase else {
            return None;
        };
        if !outcome.responded {
            return None;
        }
        if !matches!(action.kind, ActionKind::Respond | ActionKind::Outreach) {
            return None;
        }
        if self.reviewed_actions.contains(action_id) {
            return None;
        }

        Some(Action::review(
            action_id.clone(),
            action.source_messages.clone(),
            action.conversation.clone(),
            action.authority.clone(),
        ))
    }

    async fn retire_handled_triggered_intents(&self, action_id: &ActionId) {
        let Some(action) = self.registry.get(action_id) else {
            return;
        };
        if !matches!(action.kind, ActionKind::Respond) {
            return;
        }
        let super::action::Phase::Done { outcome } = &action.phase else {
            return;
        };
        if !outcome.responded {
            return;
        }
        let Some(message) = action.source_messages.first() else {
            return;
        };

        let now = chrono::Utc::now().timestamp();
        let intents = match self
            .store
            .active_intents_for_context(
                message.person.as_ref(),
                message.profile.as_ref(),
                Some(&message.conversation),
                now,
                5,
            )
            .await
        {
            Ok(intents) => intents,
            Err(e) => {
                warn!(%e, %action_id, "failed to load triggered intents after response");
                return;
            }
        };

        for intent in intents {
            if !triggered_intent_satisfied_by_inbound_response(&intent, message) {
                continue;
            }
            match self.store.complete_intent(&intent.id, now).await {
                Ok(true) => info!(
                    %action_id,
                    intent_id = %intent.id,
                    "marked handled triggered intent completed after response"
                ),
                Ok(false) => {}
                Err(e) => warn!(
                    %e,
                    %action_id,
                    intent_id = %intent.id,
                    "failed to mark handled triggered intent completed"
                ),
            }
        }
    }

    fn update_typing(
        &mut self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        sender_external_id: &str,
        typing: bool,
    ) {
        let key = (
            conversation.clone(),
            gateway_id.to_string(),
            sender_external_id.to_string(),
        );
        let Ok(mut typing_state) = self.typing.write() else {
            warn!("typing state lock poisoned");
            return;
        };
        if typing {
            typing_state.insert(key, chrono::Utc::now().timestamp());
        } else {
            typing_state.remove(&key);
        }
    }

    fn record_activity(&mut self) {
        self.last_activity_at = Some(Instant::now());
    }

    fn idle_tick_is_due(&self, elapsed_secs: f64) -> bool {
        let Some(last_activity_at) = self.last_activity_at else {
            return true;
        };
        if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
            return false;
        }
        last_activity_at.elapsed().as_secs_f64() >= elapsed_secs
    }

    fn prune_stale_typing(&self, now: i64) -> usize {
        let Ok(mut typing_state) = self.typing.write() else {
            warn!("typing state lock poisoned");
            return 0;
        };
        let before = typing_state.len();
        typing_state.retain(|_, started_at| now.saturating_sub(*started_at) <= TYPING_ACTIVE_SECS);
        before.saturating_sub(typing_state.len())
    }

    async fn flush_deferred_typing_messages(
        &self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        sender_external_id: &str,
    ) {
        let events = match self
            .store
            .pending_events_by_kind("message", TYPING_FLUSH_SCAN_LIMIT)
            .await
        {
            Ok(events) => events,
            Err(e) => {
                warn!(%e, "failed to scan pending message events after typing stopped");
                return;
            }
        };

        let now = chrono::Utc::now().timestamp();
        for event in events {
            let message = match serde_json::from_value::<InboundMessage>(event.payload.clone()) {
                Ok(message) => message,
                Err(e) => {
                    warn!(%e, event_id = %event.id, "failed to deserialize pending typing message");
                    let error = pending_typing_message_error(
                        event.last_error.as_deref(),
                        format!("failed to deserialize pending typing message: {e}"),
                    );
                    match self
                        .store
                        .mark_event_failed(&event.id, now, Some(&error))
                        .await
                    {
                        Ok(true) | Ok(false) => {}
                        Err(e) => {
                            warn!(%e, event_id = %event.id, "failed to mark pending typing message failed")
                        }
                    }
                    continue;
                }
            };

            if !typing_deferred_message_matches(
                &message,
                conversation,
                gateway_id,
                sender_external_id,
            ) {
                continue;
            }

            if !claim_and_send_persisted_event(
                &self.event_tx,
                self.store.as_ref(),
                &event.id,
                now,
                WakeEvent::Message(message),
                "typing flush",
            )
            .await
            {
                return;
            }
        }
    }

    async fn apply_message_edit(
        &self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        message_id: &str,
        content: &str,
        edited_at: i64,
    ) {
        match self
            .store
            .update_message_content_by_source(
                conversation,
                gateway_id,
                message_id,
                content,
                edited_at,
            )
            .await
        {
            Ok(true) => info!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "applied message edit"
            ),
            Ok(false) => warn!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "message edit did not match stored message"
            ),
            Err(e) => warn!(
                %e,
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "failed to apply message edit"
            ),
        }
    }

    async fn apply_message_delete(
        &self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        message_id: &str,
        deleted_at: i64,
    ) {
        match self
            .store
            .mark_message_deleted_by_source(conversation, gateway_id, message_id, deleted_at)
            .await
        {
            Ok(true) => info!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "applied message delete"
            ),
            Ok(false) => warn!(
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "message delete did not match stored message"
            ),
            Err(e) => warn!(
                %e,
                conversation = %conversation.0,
                gateway = %gateway_id,
                message_id,
                "failed to apply message delete"
            ),
        }
    }

    pub(super) fn sender_is_typing(&self, msg: &InboundMessage) -> bool {
        let key = (
            msg.conversation.clone(),
            msg.gateway_id.clone(),
            msg.sender_external_id.clone(),
        );
        self.typing.read().ok().is_some_and(|typing_state| {
            typing_state.get(&key).is_some_and(|started_at| {
                chrono::Utc::now().timestamp() - started_at <= TYPING_ACTIVE_SECS
            })
        })
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

    pub(super) async fn prune_stale_thoughts(&self, now: i64) {
        let older_than = now.saturating_sub(STALE_THOUGHT_SECS);
        match self
            .store
            .prune_stale_thoughts(
                older_than,
                STALE_THOUGHT_MAX_IMPORTANCE,
                STALE_THOUGHT_MAX_CONFIDENCE,
                STALE_THOUGHT_PRUNE_LIMIT,
            )
            .await
        {
            Ok(count) if count > 0 => {
                self.metrics.record_thoughts_pruned(count);
                info!(
                    count,
                    "pruned stale low-signal thoughts during consolidation"
                );
            }
            Ok(_) => {}
            Err(e) => warn!(%e, "failed to prune stale thoughts during consolidation"),
        }
    }

    pub(super) async fn prune_stale_memories(&self, now: i64) {
        let older_than = now.saturating_sub(STALE_MEMORY_SECS);
        match self
            .store
            .prune_stale_memories(
                now,
                older_than,
                STALE_MEMORY_MAX_IMPORTANCE,
                STALE_MEMORY_MAX_CONFIDENCE,
                STALE_MEMORY_MAX_SENSITIVITY,
                STALE_MEMORY_PRUNE_LIMIT,
            )
            .await
        {
            Ok(count) if count > 0 => {
                self.metrics.record_memories_pruned(count);
                info!(
                    count,
                    "pruned stale low-signal memories during consolidation"
                );
            }
            Ok(_) => {}
            Err(e) => warn!(%e, "failed to prune stale memories during consolidation"),
        }
    }

    pub(super) async fn prune_stale_context(&self, now: i64) {
        self.prune_stale_thoughts(now).await;
        self.prune_stale_memories(now).await;
    }
}

fn triggered_intent_satisfied_by_inbound_response(
    intent: &IntentRecord,
    msg: &InboundMessage,
) -> bool {
    if intent.kind != "triggered" || intent.status != "active" {
        return false;
    }
    if !intent_context_matches_message(intent, msg) {
        return false;
    }
    let Some(condition) = intent.condition.as_deref() else {
        return false;
    };
    is_simple_next_inbound_condition(condition)
        || content_specific_condition_matches_message(condition, msg)
}

fn intent_context_matches_message(intent: &IntentRecord, msg: &InboundMessage) -> bool {
    let targeted =
        intent.person.is_some() || intent.profile.is_some() || intent.conversation.is_some();
    if !targeted {
        let condition = intent.condition.as_deref().unwrap_or("");
        return is_generic_next_inbound_condition(condition)
            || has_content_specific_condition(&normalize_condition(condition));
    }

    if intent
        .person
        .as_ref()
        .is_some_and(|id| Some(id) != msg.person.as_ref())
    {
        return false;
    }
    if intent
        .profile
        .as_ref()
        .is_some_and(|id| Some(id) != msg.profile.as_ref())
    {
        return false;
    }
    if intent
        .conversation
        .as_ref()
        .is_some_and(|id| id != &msg.conversation)
    {
        return false;
    }
    true
}

fn is_simple_next_inbound_condition(condition: &str) -> bool {
    let condition = normalize_condition(condition);
    if condition.is_empty() || has_content_specific_condition(&condition) {
        return false;
    }
    let inbound = [
        "message", "messages", "msg", "reply", "replies", "respond", "responds", "response",
        "contact", "contacts", "ping", "pings", "dm", "chat", "talk",
    ]
    .iter()
    .any(|needle| condition.contains(needle));
    let nextish = condition.contains("next")
        || condition.contains("when they")
        || condition.contains("when this person")
        || condition.contains("when the person")
        || condition.contains("when user")
        || condition.contains("when the user");
    inbound && nextish
}

fn is_generic_next_inbound_condition(condition: &str) -> bool {
    let condition = normalize_condition(condition);
    if condition.is_empty() || has_content_specific_condition(&condition) {
        return false;
    }
    [
        "next message",
        "next reply",
        "next response",
        "next inbound",
        "next contact",
        "next ping",
        "next dm",
        "anyone messages",
        "someone messages",
        "someone replies",
    ]
    .iter()
    .any(|needle| condition.contains(needle))
}

fn has_content_specific_condition(condition: &str) -> bool {
    [
        "about ",
        "mention",
        "mentions",
        "ask ",
        "asks ",
        "asked ",
        "say ",
        "says ",
        "said ",
        "bring up",
        "brings up",
        "need ",
        "needs ",
    ]
    .iter()
    .any(|needle| condition.contains(needle))
}

fn normalize_condition(condition: &str) -> String {
    condition
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn content_specific_condition_matches_message(condition: &str, msg: &InboundMessage) -> bool {
    let condition = normalize_for_keyword_match(condition);
    let message = normalize_for_keyword_match(&msg.content);
    if condition.is_empty() || message.is_empty() {
        return false;
    }

    let Some(topic) = extract_condition_topic(&condition) else {
        return false;
    };
    if topic.is_empty() {
        return false;
    }

    let message_words = message.split_whitespace().collect::<HashSet<_>>();
    topic
        .split_whitespace()
        .all(|word| message_words.contains(word))
}

fn extract_condition_topic(condition: &str) -> Option<String> {
    for marker in [
        "mentions ",
        "mention ",
        "brings up ",
        "bring up ",
        "asks about ",
        "ask about ",
        "asked about ",
        "says ",
        "say ",
        "said ",
        "about ",
    ] {
        if let Some(rest) = condition.split(marker).nth(1) {
            let topic = rest
                .split_whitespace()
                .filter(|word| !condition_topic_stop_word(word))
                .take(6)
                .collect::<Vec<_>>()
                .join(" ");
            if !topic.is_empty() {
                return Some(topic);
            }
        }
    }
    None
}

fn condition_topic_stop_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "this"
            | "that"
            | "their"
            | "his"
            | "her"
            | "my"
            | "your"
            | "our"
            | "again"
            | "next"
            | "please"
    )
}

fn normalize_for_keyword_match(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
