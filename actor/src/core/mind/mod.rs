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
mod tests {
    use super::*;
    use crate::core::Outcome;
    use crate::core::decision::{MindDecision, MindVerdict};
    use crate::core::event::FiredIntent;
    use crate::core::handle::{SharedState, StateTask};
    use crate::identity::{Identity, Profile};
    use crate::state::{ActorState, Authority, GrowthConfig, ProactiveConsent, QuietHoursUtc};
    use crate::store::{
        EventInboxRecord, IntentRecord, Memory, MemoryKind, MemorySource, MemorySubject,
        MessageRole, RecallQuery, SqliteStore, StoredMessage, Thought, ThoughtKind,
    };
    use async_trait::async_trait;
    use gateway::{
        GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    };
    use inference::{
        AssistantMessage, Capability, ChatRequest, ChatResponse, ChatStream, FinishReason,
        InferenceEndpoint, InferenceProtocol, InferenceRouter, InferenceRouterBuilder,
        OpenAiCompatibleBridge, Reasoning, SamplingConfig, Usage,
    };
    use protocol::{
        ConversationId, GroupId, IdentityId, InboundMessage, MediaAttachment, MemoryId, PersonId,
        ProfileId,
    };
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;
    struct StateAdapter {
        state: GatewayConnectionState,
    }

    #[async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: AssistantMessage {
                    text: Some(String::new()),
                    reasoning_content: None,
                    tool_calls: vec![],
                },
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("noop bridge is not used by identity resolution tests")
        }
    }

    fn test_mind(store: Arc<dyn Store>) -> Mind {
        test_mind_with_gateway_state(store, GatewayConnectionState::Connected)
    }

    fn test_mind_with_gateway_state(
        store: Arc<dyn Store>,
        gateway_state: GatewayConnectionState,
    ) -> Mind {
        let (mind, _) = test_mind_with_gateway_state_and_event_receiver(store, gateway_state);
        mind
    }

    fn test_router() -> Arc<InferenceRouter> {
        Arc::new(
            InferenceRouterBuilder::new()
                .endpoint(InferenceEndpoint {
                    protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                    model: "noop".into(),
                    sampling: SamplingConfig::default(),
                    capabilities: vec![Capability::Chat],
                    reasoning: Reasoning::Basic,
                })
                .build()
                .unwrap(),
        )
    }

    fn test_mind_with_gateway_state_and_event_receiver(
        store: Arc<dyn Store>,
        gateway_state: GatewayConnectionState,
    ) -> (Mind, mpsc::Receiver<WakeEvent>) {
        let (_event_tx, event_rx) = mpsc::channel(4);
        let (event_tx, external_rx) = mpsc::channel(4);
        let (delta_tx, _delta_rx) = mpsc::channel(4);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let state = StateHandle::new(shared, delta_tx);
        let gateway = Arc::new(GatewayRouter::new());
        gateway.register(Arc::new(StateAdapter {
            state: gateway_state,
        }));

        (
            Mind::new(
                event_rx,
                event_tx,
                state,
                store,
                None,
                test_router(),
                gateway,
                5,
                5,
                1,
                1,
                Arc::new(ActorMetrics::default()),
            ),
            external_rx,
        )
    }

    fn test_mind_with_state_task(store: Arc<SqliteStore>) -> (Mind, tokio::task::JoinHandle<()>) {
        let (_event_tx, event_rx) = mpsc::channel(4);
        let (event_tx, _external_rx) = mpsc::channel(4);
        let (state_tx, state_rx) = mpsc::channel(4);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let state_task = StateTask::new(shared.clone(), store.clone(), state_rx, None);
        let state_join = tokio::spawn(async move {
            state_task.run().await;
        });
        let state = StateHandle::new(shared, state_tx);
        let gateway = Arc::new(GatewayRouter::new());
        gateway.register(Arc::new(StateAdapter {
            state: GatewayConnectionState::Connected,
        }));

        (
            Mind::new(
                event_rx,
                event_tx,
                state,
                store,
                None,
                test_router(),
                gateway,
                5,
                5,
                1,
                1,
                Arc::new(ActorMetrics::default()),
            ),
            state_join,
        )
    }

    #[async_trait]
    impl GatewayAdapter for StateAdapter {
        async fn connect(
            _id: String,
            _db_path: String,
            _vars: BTreeMap<String, serde_json::Value>,
            _inbound_tx: mpsc::Sender<InboundMessage>,
            _gateway_event_tx: mpsc::Sender<gateway::GatewayRuntimeEvent>,
            _media_store: Arc<media::MediaStore>,
        ) -> anyhow::Result<Self>
        where
            Self: Sized,
        {
            anyhow::bail!("state adapter is only constructed directly")
        }

        fn kind(&self) -> &str {
            "state"
        }

        fn capabilities(&self) -> GatewayCapabilities {
            GatewayCapabilities {
                content: GatewayContentCapabilities::text_only(),
                composing: true,
                read_receipts: false,
            }
        }

        fn gateway_id(&self) -> &str {
            "relay"
        }

        fn connection_state(&self) -> GatewayConnectionState {
            self.state.clone()
        }

        fn setup_instructions(&self) -> Option<protocol::GatewaySetupInstructions> {
            None
        }

        async fn send_message(
            &self,
            _external_id: &str,
            _content: &str,
            _attachments: &[MediaAttachment],
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn inbound(
        gateway_id: &str,
        sender: &str,
        display_name: &str,
        reply_target: &str,
        conversation: &str,
        group: Option<&str>,
        message_id: &str,
    ) -> InboundMessage {
        InboundMessage {
            message_id: message_id.into(),
            gateway_id: gateway_id.into(),
            sender_external_id: sender.into(),
            sender_display_name: Some(display_name.into()),
            reply_external_id: reply_target.into(),
            conversation: ConversationId(conversation.into()),
            group: group.map(|id| GroupId(id.into())),
            identity: None,
            profile: None,
            person: None,
            content: "hello".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        }
    }

    async fn append_inbound(store: &dyn Store, msg: &InboundMessage) {
        let stored = StoredMessage {
            timestamp: msg.timestamp,
            role: MessageRole::User,
            content: msg.content.clone(),
            identity: msg.identity.clone(),
            profile: msg.profile.clone(),
            person: msg.person.clone(),
            source_gateway_id: Some(msg.gateway_id.clone()),
            source_message_id: Some(msg.message_id.clone()),
            sender_external_id: Some(msg.sender_external_id.clone()),
            reply_external_id: Some(msg.reply_external_id.clone()),
            metadata: serde_json::Value::Null,
        };
        store
            .append_message(
                &msg.conversation,
                Some(&msg.gateway_id),
                msg.group.as_ref(),
                &stored,
            )
            .await
            .unwrap();
    }

    fn recent_timestamp() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn set_proactive_consent(mind: &Mind, person: &PersonId, consent: ProactiveConsent) {
        let mut actor = mind.state.shared.actor.write().unwrap();
        actor
            .bonds
            .entry(person.clone())
            .or_default()
            .proactive_consent = consent;
    }

    fn allow_proactive(mind: &Mind, person: &PersonId) {
        set_proactive_consent(mind, person, ProactiveConsent::Allowed);
    }

    fn set_unanswered_proactive_outreach(mind: &Mind, person: &PersonId) {
        let now = recent_timestamp();
        let mut actor = mind.state.shared.actor.write().unwrap();
        let rel = actor.bonds.entry(person.clone()).or_default();
        rel.last_inbound = now - 60;
        rel.last_proactive_outbound = now - 30;
        rel.proactive_outbound_count = 1;
    }

    fn fill_capacity_with_running_responses(mind: &mut Mind) {
        for idx in 0..5 {
            let conversation = format!("relay:running-{idx}");
            let msg = inbound(
                "relay",
                &format!("running-{idx}"),
                "Sam",
                &format!("running-{idx}"),
                &conversation,
                None,
                &format!("running-msg-{idx}"),
            );
            let action = Action::respond(
                vec![msg.clone()],
                msg.conversation.clone(),
                Authority::Default,
                None,
            );
            let id = mind.registry.schedule(action);
            mind.registry.launch(&id).expect("action launches");
        }
    }

    #[tokio::test]
    async fn first_relay_contact_relationship_config_is_journaled() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (mind, state_join) = test_mind_with_state_task(store.clone());
        let mut msg = inbound(
            "relay",
            "sam-local",
            "Sam",
            "sam-local",
            "relay:sam-local",
            None,
            "relay-chosen_person-msg-1",
        );

        ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

        let person = msg
            .person
            .clone()
            .expect("relay contact resolves to a person");
        {
            let actor = mind.state.read_state();
            assert_eq!(actor.bonds[&person].authority, Authority::ChosenPerson);
        }
        let records = store.state_journal_after(None, 10).await.unwrap();
        let relationship_record = records
            .iter()
            .find(|record| record.kind == "relationship_config")
            .expect("relationship config journal record");
        assert_eq!(
            relationship_record.payload["person_id"].as_str(),
            Some(person.0.as_str())
        );
        assert_eq!(
            relationship_record.payload["authority"].as_str(),
            Some("chosen_person")
        );

        drop(mind);
        state_join.await.unwrap();
    }

    #[tokio::test]
    async fn discord_channel_resolves_authors_as_distinct_profiles_in_one_conversation() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());

        let mut alice = inbound(
            "discord",
            "author-a",
            "Alice",
            "channel-1",
            "discord:channel-1",
            Some("discord:guild-1"),
            "msg-a",
        );
        let mut bob = inbound(
            "discord",
            "author-b",
            "Bob",
            "channel-1",
            "discord:channel-1",
            Some("discord:guild-1"),
            "msg-b",
        );

        ingest::resolve_person(&mind.state, &mind.store, &mut alice).await;
        ingest::resolve_person(&mind.state, &mind.store, &mut bob).await;
        append_inbound(store.as_ref(), &alice).await;
        append_inbound(store.as_ref(), &bob).await;

        assert_ne!(alice.identity, bob.identity);
        assert_ne!(alice.profile, bob.profile);
        assert_eq!(alice.conversation, bob.conversation);
        assert!(
            store
                .resolve_identity("discord", "channel-1")
                .await
                .unwrap()
                .is_none()
        );

        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(
            conversations[0].id,
            ConversationId("discord:channel-1".into())
        );
        assert_eq!(
            conversations[0].group.as_ref(),
            Some(&GroupId("discord:guild-1".into()))
        );
        let group = store
            .get_group(&GroupId("discord:guild-1".into()))
            .await
            .unwrap()
            .unwrap();
        assert!(group.members.contains(alice.person.as_ref().unwrap()));
        assert!(group.members.contains(bob.person.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn whatsapp_group_sender_memories_are_profile_scoped() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());

        let mut alice = inbound(
            "whatsapp",
            "alice@s.whatsapp.net",
            "Alice",
            "family@g.us",
            "whatsapp:family@g.us",
            Some("family@g.us"),
            "wa-msg-a",
        );
        let mut bob = inbound(
            "whatsapp",
            "bob@s.whatsapp.net",
            "Bob",
            "family@g.us",
            "whatsapp:family@g.us",
            Some("family@g.us"),
            "wa-msg-b",
        );

        ingest::resolve_person(&mind.state, &mind.store, &mut alice).await;
        ingest::resolve_person(&mind.state, &mind.store, &mut bob).await;
        append_inbound(store.as_ref(), &alice).await;
        append_inbound(store.as_ref(), &bob).await;

        let alice_profile = alice.profile.clone().unwrap();
        let bob_profile = bob.profile.clone().unwrap();
        assert_ne!(alice_profile, bob_profile);

        store
            .store_memory(&Memory {
                id: MemoryId("memory-alice".into()),
                kind: MemoryKind::Semantic,
                content: "Alice prefers concise deployment updates.".into(),
                source: MemorySource::Conversation {
                    conversation_id: alice.conversation.clone(),
                    identity_id: alice.identity.clone(),
                    profile_id: Some(alice_profile.clone()),
                    person_id: alice.person.clone(),
                    message_id: Some(alice.message_id.clone()),
                },
                importance: 0.8,
                sensitivity: 0.0,
                emotional_valence: 0.0,
                created_at: 1000,
                accessed_at: 1000,
                access_count: 0,
                tags: vec![],
                subjects: vec![MemorySubject::profile(
                    alice_profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                embedding: None,
                ..Memory::default()
            })
            .await
            .unwrap();

        let alice_memories = store
            .recall(&RecallQuery::by_text("deployment", 10).with_profile(alice_profile))
            .await
            .unwrap();
        let bob_memories = store
            .recall(&RecallQuery::by_text("deployment", 10).with_profile(bob_profile))
            .await
            .unwrap();

        assert_eq!(alice_memories.len(), 1);
        assert!(bob_memories.is_empty());

        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(
            conversations[0].group.as_ref(),
            Some(&GroupId("family@g.us".into()))
        );
        let group = store
            .get_group(&GroupId("family@g.us".into()))
            .await
            .unwrap()
            .unwrap();
        assert!(group.members.contains(alice.person.as_ref().unwrap()));
        assert!(group.members.contains(bob.person.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn existing_gateway_identity_refreshes_observed_display_name() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let identity = Identity {
            id: IdentityId("identity-local".into()),
            gateway_id: "relay".into(),
            external_id: "local".into(),
            display_name: Some("Old Sam".into()),
            metadata: None,
            created_at: 900,
            last_seen_at: 900,
        };
        let profile = Profile {
            id: ProfileId("profile-local".into()),
            display_name: None,
            summary: None,
            comm_style: None,
            first_seen: 900,
            last_seen: 900,
            created_at: 900,
            updated_at: 900,
        };
        store.add_identity(&identity).await.unwrap();
        store.add_profile(&profile).await.unwrap();
        store
            .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
            .await
            .unwrap();

        let mut msg = inbound(
            "relay",
            "local",
            "New Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

        let refreshed_identity = store.get_identity(&identity.id).await.unwrap().unwrap();
        let refreshed_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
        assert_eq!(refreshed_identity.display_name.as_deref(), Some("New Sam"));
        assert_eq!(refreshed_profile.display_name.as_deref(), Some("New Sam"));
        let observations = store
            .display_name_observations(&identity.id, 10)
            .await
            .unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].display_name, "New Sam");
        assert_eq!(
            observations[0].source_message_id.as_deref(),
            Some("local-msg-1")
        );
        assert_eq!(observations[0].profile.as_ref(), Some(&profile.id));

        store
            .update_profile(&profile.id, Some("Preferred Sam"), None)
            .await
            .unwrap();
        let mut msg = inbound(
            "relay",
            "local",
            "Newest Sam",
            "local",
            "relay:local",
            None,
            "local-msg-2",
        );
        ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

        let refreshed_identity = store.get_identity(&identity.id).await.unwrap().unwrap();
        let preserved_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
        assert_eq!(
            refreshed_identity.display_name.as_deref(),
            Some("Newest Sam")
        );
        assert_eq!(
            preserved_profile.display_name.as_deref(),
            Some("Preferred Sam")
        );
        let observations = store
            .display_name_observations(&identity.id, 10)
            .await
            .unwrap();
        assert_eq!(
            observations
                .iter()
                .map(|observation| observation.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["New Sam", "Newest Sam"]
        );

        let mut duplicate_msg = inbound(
            "relay",
            "local",
            "Newest Sam",
            "local",
            "relay:local",
            None,
            "local-msg-2",
        );
        ingest::resolve_person(&mind.state, &mind.store, &mut duplicate_msg).await;
        let observations = store
            .display_name_observations(&identity.id, 10)
            .await
            .unwrap();
        assert_eq!(observations.len(), 2);
    }

    #[tokio::test]
    async fn existing_gateway_identity_refreshes_auto_profile_display_name() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let identity = Identity {
            id: IdentityId("identity-auto-name".into()),
            gateway_id: "relay".into(),
            external_id: "local".into(),
            display_name: Some("Old Sam".into()),
            metadata: None,
            created_at: 900,
            last_seen_at: 900,
        };
        let profile = Profile {
            id: ProfileId("profile-auto-name".into()),
            display_name: Some("Old Sam".into()),
            summary: None,
            comm_style: None,
            first_seen: 900,
            last_seen: 900,
            created_at: 900,
            updated_at: 900,
        };
        store.add_identity(&identity).await.unwrap();
        store.add_profile(&profile).await.unwrap();
        store
            .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
            .await
            .unwrap();

        let mut msg = inbound(
            "relay",
            "local",
            "New Sam",
            "local",
            "relay:local",
            None,
            "local-msg-auto-name",
        );
        ingest::resolve_person(&mind.state, &mind.store, &mut msg).await;

        let refreshed_identity = store.get_identity(&identity.id).await.unwrap().unwrap();
        let refreshed_profile = store.get_profile(&profile.id).await.unwrap().unwrap();
        assert_eq!(refreshed_identity.display_name.as_deref(), Some("New Sam"));
        assert_eq!(refreshed_profile.display_name.as_deref(), Some("New Sam"));
    }

    #[test]
    fn successful_visible_action_builds_one_post_turn_review() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        let action = Action::respond(
            vec![msg.clone()],
            msg.conversation.clone(),
            Authority::Default,
            None,
        );
        let injected = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-2",
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: true,
                attempted_send: true,
                review_messages: vec![injected.clone()],
                attempts: 1,
                ..Outcome::default()
            },
        );

        let review = mind
            .build_post_turn_review(&id)
            .expect("responded action should produce review");
        assert!(matches!(review.kind, ActionKind::Review));
        assert!(!review.kind.expects_response());
        assert_eq!(
            review.conversation,
            Some(ConversationId("relay:local".into()))
        );
        assert_eq!(review.source_messages.len(), 2);
        assert_eq!(review.source_messages[0].message_id, msg.message_id);
        assert_eq!(review.source_messages[1].message_id, injected.message_id);

        mind.reviewed_actions.insert(id.clone());
        assert!(mind.build_post_turn_review(&id).is_none());
    }

    #[test]
    fn successful_outreach_action_builds_post_turn_review() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        let action = Action::outreach(
            "Check in about the deployment".into(),
            Some(ConversationId("relay:local".into())),
            Authority::Default,
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: true,
                attempted_send: true,
                attempts: 1,
                ..Outcome::default()
            },
        );

        let review = mind
            .build_post_turn_review(&id)
            .expect("successful outreach should produce review");

        assert!(matches!(review.kind, ActionKind::Review));
        assert_eq!(
            review.conversation,
            Some(ConversationId("relay:local".into()))
        );
        assert_eq!(review.authority, Authority::Default);
        assert!(review.source_messages.is_empty());
    }

    #[test]
    fn failed_visible_action_does_not_build_post_turn_review() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        let action = Action::respond(
            vec![msg],
            ConversationId("relay:local".into()),
            Authority::Default,
            None,
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: false,
                attempted_send: false,
                attempts: 1,
                ..Outcome::default()
            },
        );

        assert!(mind.build_post_turn_review(&id).is_none());
    }

    #[tokio::test]
    async fn successful_response_retires_simple_next_message_triggered_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let profile = ProfileId("profile-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        msg.person = Some(person.clone());
        msg.profile = Some(profile.clone());
        let now = recent_timestamp();

        store
            .create_intent(&IntentRecord {
                id: "intent-next-message".into(),
                kind: "triggered".into(),
                status: "active".into(),
                task: "Ask how the deployment went".into(),
                person: Some(person.clone()),
                profile: Some(profile.clone()),
                conversation: Some(msg.conversation.clone()),
                fire_at: None,
                condition: Some("next time Sam messages".into()),
                recurrence: None,
                priority: 80,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now,
                updated_at: now,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        store
            .create_intent(&IntentRecord {
                id: "intent-specific-condition".into(),
                kind: "triggered".into(),
                status: "active".into(),
                task: "Ask about the deployment only if Sam brings it up".into(),
                person: Some(person.clone()),
                profile: Some(profile),
                conversation: Some(msg.conversation.clone()),
                fire_at: None,
                condition: Some("when Sam mentions deployment".into()),
                recurrence: None,
                priority: 70,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now,
                updated_at: now,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let action = Action::respond(
            vec![msg.clone()],
            msg.conversation.clone(),
            Authority::Default,
            None,
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: true,
                attempted_send: true,
                attempts: 1,
                ..Outcome::default()
            },
        );

        mind.retire_handled_triggered_intents(&id).await;

        let retired = store
            .get_intent("intent-next-message")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retired.status, "completed");
        assert!(retired.last_fired_at.is_none());

        let still_active = store
            .get_intent("intent-specific-condition")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_active.status, "active");
        assert!(still_active.last_fired_at.is_none());
    }

    #[tokio::test]
    async fn successful_response_retires_matched_content_triggered_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let profile = ProfileId("profile-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        msg.content = "Deployment finished, but the rollback notes are messy.".into();
        msg.person = Some(person.clone());
        msg.profile = Some(profile.clone());
        let now = recent_timestamp();

        store
            .create_intent(&IntentRecord {
                id: "intent-deployment-condition".into(),
                kind: "triggered".into(),
                status: "active".into(),
                task: "Ask about the deployment only if Sam brings it up".into(),
                person: Some(person.clone()),
                profile: Some(profile.clone()),
                conversation: Some(msg.conversation.clone()),
                fire_at: None,
                condition: Some("when Sam mentions deployment".into()),
                recurrence: None,
                priority: 70,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now,
                updated_at: now,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        store
            .create_intent(&IntentRecord {
                id: "intent-budget-condition".into(),
                kind: "triggered".into(),
                status: "active".into(),
                task: "Ask about budget only if Sam brings it up".into(),
                person: Some(person.clone()),
                profile: Some(profile),
                conversation: Some(msg.conversation.clone()),
                fire_at: None,
                condition: Some("when Sam asks about budget".into()),
                recurrence: None,
                priority: 60,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now,
                updated_at: now,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let action = Action::respond(
            vec![msg.clone()],
            msg.conversation.clone(),
            Authority::Default,
            None,
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: true,
                attempted_send: true,
                attempts: 1,
                ..Outcome::default()
            },
        );

        mind.retire_handled_triggered_intents(&id).await;

        let matched = store
            .get_intent("intent-deployment-condition")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(matched.status, "completed");
        assert!(matched.last_fired_at.is_none());

        let unmatched = store
            .get_intent("intent-budget-condition")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unmatched.status, "active");
        assert!(unmatched.last_fired_at.is_none());
    }

    #[tokio::test]
    async fn successful_outreach_completes_fired_source_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let conversation = ConversationId("relay:local".into());
        let now = recent_timestamp();
        store
            .create_intent(&IntentRecord {
                id: "intent-outreach".into(),
                kind: "proactive".into(),
                status: "fired".into(),
                task: "Check in about the deployment".into(),
                person: None,
                profile: None,
                conversation: Some(conversation.clone()),
                fire_at: Some(now - 60),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now - 120,
                updated_at: now - 60,
                last_fired_at: Some(now - 60),
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let mut mind = mind;
        let action = Action::outreach_with_source_intent(
            "Check in about the deployment".into(),
            Some(conversation),
            Authority::Default,
            Some("intent-outreach".into()),
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: true,
                attempted_send: true,
                attempts: 1,
                ..Outcome::default()
            },
        );

        mind.complete_successful_outreach_source_intent(&id).await;

        let completed = store.get_intent("intent-outreach").await.unwrap().unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.last_fired_at, Some(now - 60));
    }

    #[tokio::test]
    async fn successful_recurring_outreach_keeps_active_source_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store.clone());
        let conversation = ConversationId("relay:local".into());
        let now = recent_timestamp();
        store
            .create_intent(&IntentRecord {
                id: "intent-recurring-outreach".into(),
                kind: "proactive".into(),
                status: "active".into(),
                task: "Weekly check-in".into(),
                person: None,
                profile: None,
                conversation: Some(conversation.clone()),
                fire_at: Some(now + 60 * 60),
                condition: None,
                recurrence: Some("weekly".into()),
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now - 120,
                updated_at: now - 60,
                last_fired_at: Some(now - 60),
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let action = Action::outreach_with_source_intent(
            "Weekly check-in".into(),
            Some(conversation),
            Authority::Default,
            Some("intent-recurring-outreach".into()),
        );
        let id = mind.registry.schedule(action);
        mind.registry.complete(
            &id,
            Outcome {
                responded: true,
                attempted_send: true,
                attempts: 1,
                ..Outcome::default()
            },
        );

        mind.complete_successful_outreach_source_intent(&id).await;

        let active = store
            .get_intent("intent-recurring-outreach")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.status, "active");
        assert_eq!(active.recurrence.as_deref(), Some("weekly"));
        assert_eq!(active.last_fired_at, Some(now - 60));
    }

    #[tokio::test]
    async fn defer_verdict_reemits_message_with_bounded_count() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );

        let decision = mind
            .build_decision(
                MindVerdict::Defer { delay_secs: 999 },
                &WakeEvent::Message(msg),
            )
            .await;

        match decision {
            MindDecision::DeferMessage(deferred, delay_secs) => {
                assert_eq!(delay_secs, 300);
                assert_eq!(deferred.metadata["mind_defer_count"], 1);
            }
            _ => panic!("expected deferred message"),
        }
    }

    #[tokio::test]
    async fn defer_verdict_reemits_intent_with_chosen_person_approval_and_bounded_count() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);
        let intent = FiredIntent {
            id: "intent-chosen-person-approved".into(),
            task: "Follow up after the deploy".into(),
            conversation: Some(ConversationId("relay:local".into())),
            person: Some(PersonId("person-sam".into())),
            scheduled_at: None,
            chosen_person_approved: true,
            defer_count: 1,
        };

        let decision = mind
            .build_decision(
                MindVerdict::Defer { delay_secs: 999 },
                &WakeEvent::IntentFired(intent),
            )
            .await;

        match decision {
            MindDecision::DeferIntent(deferred, delay_secs) => {
                assert_eq!(delay_secs, 300);
                assert_eq!(deferred.id, "intent-chosen-person-approved");
                assert!(deferred.chosen_person_approved);
                assert_eq!(deferred.defer_count, 2);
            }
            _ => panic!("expected deferred intent"),
        }
    }

    #[tokio::test]
    async fn defer_verdict_reemits_consolidation_due() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);

        let decision = mind
            .build_decision(
                MindVerdict::Defer { delay_secs: 999 },
                &WakeEvent::ConsolidationDue,
            )
            .await;

        match decision {
            MindDecision::DeferConsolidation(delay_secs) => assert_eq!(delay_secs, 300),
            _ => panic!("expected deferred consolidation"),
        }
    }

    #[tokio::test]
    async fn at_capacity_defers_new_message_instead_of_dropping() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        fill_capacity_with_running_responses(&mut mind);
        let msg = inbound(
            "relay",
            "incoming",
            "Sam",
            "incoming",
            "relay:incoming",
            None,
            "incoming-msg-1",
        );

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::Message(msg),
            )
            .await;

        match decision {
            MindDecision::DeferMessage(deferred, delay_secs) => {
                assert_eq!(delay_secs, 15);
                assert_eq!(deferred.metadata["mind_defer_count"], 1);
            }
            _ => panic!("expected deferred message at capacity"),
        }
    }

    #[tokio::test]
    async fn typing_sender_defers_fresh_response_until_typing_stops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        let person = PersonId("sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        msg.person = Some(person.clone());
        mind.update_typing(
            &msg.conversation,
            &msg.gateway_id,
            &msg.sender_external_id,
            true,
        );

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::Message(msg.clone()),
            )
            .await;
        match decision {
            MindDecision::DeferMessage(deferred, delay_secs) => {
                assert_eq!(delay_secs, 5);
                assert_eq!(deferred.metadata["mind_defer_count"], 1);
                assert_eq!(deferred.metadata["mind_defer_reason"], "typing");
            }
            _ => panic!("expected deferred message while sender is typing"),
        }

        mind.update_typing(
            &msg.conversation,
            &msg.gateway_id,
            &msg.sender_external_id,
            false,
        );
        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::Message(msg),
            )
            .await;
        assert!(matches!(decision, MindDecision::Spawn(_)));
    }

    #[tokio::test]
    async fn typing_stop_flushes_matching_deferred_typing_message() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (mind, mut flushed_rx) = test_mind_with_gateway_state_and_event_receiver(
            store.clone(),
            GatewayConnectionState::Connected,
        );
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        msg.metadata = serde_json::json!({
            "mind_defer_count": 1,
            "mind_defer_reason": "typing",
        });
        let now = chrono::Utc::now().timestamp();
        store
            .enqueue_event(&EventInboxRecord {
                id: "typing-event".into(),
                kind: "message".into(),
                payload: serde_json::to_value(&msg).unwrap(),
                status: "pending".into(),
                due_at: now + 300,
                attempts: 0,
                dedupe_key: Some("message:relay:local-msg-1:1".into()),
                created_at: now,
                updated_at: now,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        mind.flush_deferred_typing_messages(
            &msg.conversation,
            &msg.gateway_id,
            &msg.sender_external_id,
        )
        .await;

        let flushed = tokio::time::timeout(std::time::Duration::from_secs(1), flushed_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match flushed {
            WakeEvent::Message(flushed_msg) => assert_eq!(flushed_msg.message_id, "local-msg-1"),
            _ => panic!("expected flushed message"),
        }
        assert!(store.due_events(now + 301, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn typing_stop_marks_malformed_pending_message_failed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (mind, mut flushed_rx) = test_mind_with_gateway_state_and_event_receiver(
            store.clone(),
            GatewayConnectionState::Connected,
        );
        let now = chrono::Utc::now().timestamp();
        store
            .enqueue_event(&EventInboxRecord {
                id: "malformed-typing-event".into(),
                kind: "message".into(),
                payload: serde_json::json!({"malformed": true}),
                status: "pending".into(),
                due_at: now + 300,
                attempts: 0,
                dedupe_key: Some("message:relay:malformed:1".into()),
                created_at: now,
                updated_at: now,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        mind.flush_deferred_typing_messages(
            &ConversationId("relay:local".into()),
            "relay",
            "local",
        )
        .await;

        assert!(flushed_rx.try_recv().is_err());
        assert!(
            store
                .pending_events_by_kind("message", 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn typing_stop_keeps_pending_message_when_flush_channel_is_closed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (mind, flushed_rx) = test_mind_with_gateway_state_and_event_receiver(
            store.clone(),
            GatewayConnectionState::Connected,
        );
        drop(flushed_rx);
        let now = chrono::Utc::now().timestamp();
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        msg.metadata = serde_json::json!({
            "mind_defer_reason": "typing",
        });
        store
            .enqueue_event(&EventInboxRecord {
                id: "typing-event".into(),
                kind: "message".into(),
                payload: serde_json::to_value(&msg).unwrap(),
                status: "pending".into(),
                due_at: now + 300,
                attempts: 0,
                dedupe_key: Some("message:relay:local:local-msg-1".into()),
                created_at: now,
                updated_at: now,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        mind.flush_deferred_typing_messages(
            &ConversationId("relay:local".into()),
            "relay",
            "local",
        )
        .await;

        let pending = store.pending_events_by_kind("message", 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "typing-event");
    }

    #[tokio::test]
    async fn idle_tick_prunes_stale_typing_state() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);
        let conversation = ConversationId("relay:local".into());
        let stale_key = (
            conversation.clone(),
            "relay".to_string(),
            "stale-sender".to_string(),
        );
        let active_key = (
            conversation.clone(),
            "relay".to_string(),
            "active-sender".to_string(),
        );
        let future_key = (
            conversation,
            "relay".to_string(),
            "future-sender".to_string(),
        );
        let now = recent_timestamp();
        {
            let mut typing = mind.typing.write().unwrap();
            typing.insert(stale_key.clone(), now - TYPING_ACTIVE_SECS - 1);
            typing.insert(active_key.clone(), now);
            typing.insert(future_key.clone(), now + 60);
        }

        let pruned = mind.prune_stale_typing(now);

        assert_eq!(pruned, 1);
        let typing = mind.typing.read().unwrap();
        assert!(!typing.contains_key(&stale_key));
        assert!(typing.contains_key(&active_key));
        assert!(typing.contains_key(&future_key));
    }

    #[test]
    fn idle_tick_waits_for_actual_inactivity_window() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);

        assert!(mind.idle_tick_is_due(300.0));
        mind.record_activity();
        assert!(!mind.idle_tick_is_due(300.0));
        mind.last_activity_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(600));
        assert!(mind.idle_tick_is_due(300.0));
        assert!(!mind.idle_tick_is_due(0.0));

        assert!(!event_counts_as_activity(&WakeEvent::IdleTick {
            elapsed_secs: 300.0,
        }));
        assert!(event_counts_as_activity(&WakeEvent::TypingUpdate {
            conversation: ConversationId("relay:local".into()),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            typing: true,
        }));
    }

    #[tokio::test]
    async fn message_revision_events_update_stored_conversation_history() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let conv = ConversationId("relay:local".into());
        store
            .append_message(
                &conv,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "before edit".into(),
                    identity: None,
                    profile: None,
                    person: None,
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-1".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();

        mind.apply_message_edit(&conv, "relay", "msg-1", "after edit", 1100)
            .await;
        let messages = store.get_messages(&conv, 10, None).await.unwrap();
        assert_eq!(messages[0].content, "after edit");
        assert_eq!(messages[0].metadata["edited_at"], 1100);

        mind.apply_message_delete(&conv, "relay", "msg-1", 1200)
            .await;
        let messages = store.get_messages(&conv, 10, None).await.unwrap();
        assert_eq!(messages[0].content, "[message deleted]");
        assert_eq!(messages[0].metadata["deleted_at"], 1200);
    }

    #[tokio::test]
    async fn consolidation_due_spawns_consolidate_action() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::ConsolidationDue,
            )
            .await;

        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Consolidate));
                assert_eq!(action.priority, ActionKind::Consolidate.default_priority());
            }
            _ => panic!("expected consolidation action"),
        }
    }

    #[tokio::test]
    async fn at_capacity_defers_consolidation_due_instead_of_dropping() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        fill_capacity_with_running_responses(&mut mind);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::ConsolidationDue,
            )
            .await;

        match decision {
            MindDecision::DeferConsolidation(delay_secs) => {
                assert_eq!(delay_secs, 300);
            }
            _ => panic!("expected deferred consolidation at capacity"),
        }
    }

    #[tokio::test]
    async fn deferred_consolidation_is_persisted_for_scheduler_retry() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store.clone());

        mind.execute_decision(MindDecision::DeferConsolidation(300))
            .await;

        let pending = store
            .pending_events_by_kind("consolidation_due", 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].dedupe_key.as_deref(), Some("consolidation-due"));
        assert_eq!(pending[0].attempts, 0);
        assert_eq!(mind.metrics.snapshot().events_deferred, 1);
    }

    #[tokio::test]
    async fn deferred_message_timer_keeps_pending_event_when_actor_channel_is_closed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store.clone());
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-deferred-closed-channel",
        );

        mind.execute_decision(MindDecision::DeferMessage(msg, 0))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let pending = store.pending_events_by_kind("message", 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, "pending");
        assert_eq!(pending[0].attempts, 0);
    }

    #[tokio::test]
    async fn mind_metrics_track_decisions_and_action_queue() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );

        mind.execute_decision(MindDecision::Drop).await;
        mind.execute_decision(MindDecision::Inject(
            ActionId("missing-action".into()),
            msg.clone(),
        ))
        .await;
        mind.execute_decision(MindDecision::DeferMessage(msg, 300))
            .await;
        let queued_id = mind.schedule_action(Action::ruminate());
        let snapshot = mind.metrics.snapshot();
        assert_eq!(snapshot.events_dropped, 1);
        assert_eq!(snapshot.events_deferred, 1);
        assert_eq!(snapshot.injection_failures, 1);
        assert_eq!(snapshot.actions_spawned, 1);
        assert_eq!(snapshot.action_queue_length, 1);
        assert_eq!(snapshot.running_actions, 0);

        mind.registry.launch(&queued_id).expect("action launches");
        mind.refresh_registry_metrics();
        let snapshot = mind.metrics.snapshot();
        assert_eq!(snapshot.action_queue_length, 0);
        assert_eq!(snapshot.running_actions, 1);
    }

    #[tokio::test]
    async fn failed_running_action_injection_requeues_message_and_skips_dead_target() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (mut mind, mut external_rx) = test_mind_with_gateway_state_and_event_receiver(
            store,
            GatewayConnectionState::Connected,
        );
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-injected",
        );
        let id = mind.schedule_action(Action::respond(
            vec![msg.clone()],
            msg.conversation.clone(),
            Authority::Default,
            None,
        ));
        let launch = mind.registry.launch(&id).expect("action launches");
        drop(launch);

        mind.execute_decision(MindDecision::Inject(id.clone(), msg.clone()))
            .await;

        let requeued = match external_rx.recv().await.expect("message requeued") {
            WakeEvent::Message(msg) => msg,
            _ => panic!("expected requeued message"),
        };
        assert!(message_skips_injection_target(&requeued, &id));

        let decision = mind
            .respond_to(&WakeEvent::Message(requeued.clone()), None)
            .await;
        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Respond));
                assert_eq!(action.source_messages[0].message_id, requeued.message_id);
            }
            _ => panic!("expected fresh respond action instead of retrying dead injection"),
        }
    }

    #[tokio::test]
    async fn consolidation_prunes_stale_low_signal_thoughts() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let now = recent_timestamp();
        for (idx, age_secs, content, importance, confidence) in [
            (
                1,
                STALE_THOUGHT_SECS + 10,
                "old low signal thought",
                0.1,
                0.2,
            ),
            (
                2,
                STALE_THOUGHT_SECS + 10,
                "old important thought",
                0.9,
                0.2,
            ),
            (3, 10, "recent low signal thought", 0.1, 0.2),
        ] {
            store
                .log_thought(&Thought {
                    timestamp: now - age_secs,
                    kind: ThoughtKind::Observation,
                    content: content.into(),
                    importance,
                    confidence,
                    action_id: Some(format!("action-{idx}")),
                    memories_accessed: vec![],
                    subjects: vec![],
                })
                .await
                .unwrap();
        }

        mind.prune_stale_thoughts(now).await;

        let thoughts = store.recent_thoughts(10).await.unwrap();
        let contents = thoughts
            .iter()
            .map(|thought| thought.content.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            contents,
            vec!["old important thought", "recent low signal thought"]
        );
        assert_eq!(mind.metrics.snapshot().thoughts_pruned, 1);
    }

    #[tokio::test]
    async fn consolidation_prunes_stale_low_signal_memories() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let now = recent_timestamp();
        store
            .store_memory(&Memory {
                id: MemoryId("expired-memory".into()),
                kind: MemoryKind::Semantic,
                content: "Temporary low-signal note".into(),
                source: MemorySource::Reflection,
                importance: 0.1,
                confidence: 0.2,
                sensitivity: 0.1,
                created_at: now - STALE_MEMORY_SECS,
                accessed_at: now - STALE_MEMORY_SECS,
                expires_at: Some(now - 1),
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("important-expired-memory".into()),
                kind: MemoryKind::Semantic,
                content: "Important expired note".into(),
                source: MemorySource::Reflection,
                importance: 0.9,
                confidence: 0.2,
                sensitivity: 0.1,
                created_at: now - STALE_MEMORY_SECS,
                accessed_at: now - STALE_MEMORY_SECS,
                expires_at: Some(now - 1),
                ..Memory::default()
            })
            .await
            .unwrap();

        mind.prune_stale_memories(now).await;

        assert!(
            store
                .get_memory(&MemoryId("expired-memory".into()))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_memory(&MemoryId("important-expired-memory".into()))
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(mind.metrics.snapshot().memories_pruned, 1);
    }

    #[tokio::test]
    async fn at_capacity_defers_proactive_intent_instead_of_dropping() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        fill_capacity_with_running_responses(&mut mind);
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person);
        msg.timestamp = recent_timestamp();
        append_inbound(mind.store.as_ref(), &msg).await;
        allow_proactive(&mind, msg.person.as_ref().unwrap());

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Check in".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::DeferIntent(intent, delay_secs) => {
                assert_eq!(delay_secs, 60);
                assert_eq!(intent.defer_count, 1);
            }
            _ => panic!("expected deferred intent at capacity"),
        }
    }

    #[tokio::test]
    async fn proactive_intent_drops_when_last_visible_message_is_assistant() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let conversation = ConversationId("relay:local".into());
        let person = PersonId("person-sam".into());
        let now = recent_timestamp();
        allow_proactive(&mind, &person);
        store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: now - 10,
                    role: MessageRole::User,
                    content: "ping me later".into(),
                    identity: None,
                    profile: None,
                    person: Some(person.clone()),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-1".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: now - 5,
                    role: MessageRole::Assistant,
                    content: "will do".into(),
                    identity: None,
                    profile: None,
                    person: None,
                    source_gateway_id: None,
                    source_message_id: None,
                    sender_external_id: None,
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Check in".into(),
                    conversation: Some(conversation),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn proactive_intent_drops_when_target_replied_after_scheduling() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let conversation = ConversationId("relay:local".into());
        let person = PersonId("person-sam".into());
        let now = recent_timestamp();
        allow_proactive(&mind, &person);

        store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: now - 300,
                    role: MessageRole::User,
                    content: "can you check in later?".into(),
                    identity: None,
                    profile: None,
                    person: Some(person.clone()),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-before-intent".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .create_intent(&IntentRecord {
                id: "intent-obsolete-followup".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Check in with Sam".into(),
                person: Some(person.clone()),
                profile: None,
                conversation: Some(conversation.clone()),
                fire_at: Some(now - 10),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: Some("review-action".into()),
                source_memory: None,
                created_at: now - 240,
                updated_at: now - 240,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: now - 60,
                    role: MessageRole::User,
                    content: "actually, I sorted it out".into(),
                    identity: None,
                    profile: None,
                    person: Some(person.clone()),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-after-intent".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        assert!(
            store
                .mark_intent_fired("intent-obsolete-followup", now)
                .await
                .unwrap()
        );

        let event = WakeEvent::IntentFired(FiredIntent {
            id: "intent-obsolete-followup".into(),
            task: "Check in with Sam".into(),
            conversation: Some(conversation),
            person: Some(person),
            scheduled_at: Some(now - 240),
            chosen_person_approved: false,
            defer_count: 0,
        });
        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &event,
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
        mind.retire_dropped_fired_intent(&event, &decision).await;
        let stored = store
            .get_intent("intent-obsolete-followup")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "completed");
    }

    #[tokio::test]
    async fn proactive_intent_uses_latest_schedule_time_for_reply_obsolescence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let conversation = ConversationId("relay:local".into());
        let person = PersonId("person-sam".into());
        let now = recent_timestamp();
        allow_proactive(&mind, &person);

        store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: now - 120,
                    role: MessageRole::User,
                    content: "I replied before you rescheduled the check-in.".into(),
                    identity: None,
                    profile: None,
                    person: Some(person.clone()),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-before-reschedule".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .create_intent(&IntentRecord {
                id: "intent-rescheduled-followup".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Check in after Sam's reply".into(),
                person: Some(person.clone()),
                profile: None,
                conversation: Some(conversation.clone()),
                fire_at: Some(now - 10),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: Some("review-action".into()),
                source_memory: None,
                created_at: now - 600,
                updated_at: now - 30,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        assert!(
            store
                .mark_intent_fired("intent-rescheduled-followup", now)
                .await
                .unwrap()
        );

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-rescheduled-followup".into(),
                    task: "Check in after Sam's reply".into(),
                    conversation: Some(conversation),
                    person: Some(person),
                    scheduled_at: Some(now - 30),
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(
            matches!(decision, MindDecision::Spawn(action) if matches!(action.kind, ActionKind::Outreach))
        );
    }

    #[tokio::test]
    async fn proactive_intent_without_conversation_uses_last_person_conversation() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        allow_proactive(&mind, &person);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: None,
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Outreach));
                assert_eq!(
                    action.conversation,
                    Some(ConversationId("relay:local".into()))
                );
                assert_eq!(action.source_intent.as_deref(), Some("intent-1"));
            }
            _ => panic!("expected outreach spawn with inferred conversation"),
        }
    }

    #[tokio::test]
    async fn proactive_intent_without_conversation_uses_channel_preference_when_available() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let now = recent_timestamp();
        let mut preferred = inbound(
            "relay",
            "preferred",
            "Sam",
            "preferred",
            "relay:preferred",
            None,
            "msg-preferred",
        );
        preferred.person = Some(person.clone());
        preferred.timestamp = now - 120;
        let mut recent = inbound(
            "relay",
            "recent",
            "Sam",
            "recent",
            "relay:recent",
            None,
            "msg-recent",
        );
        recent.person = Some(person.clone());
        recent.timestamp = now - 10;
        append_inbound(store.as_ref(), &preferred).await;
        append_inbound(store.as_ref(), &recent).await;
        {
            let mut actor = mind.state.shared.actor.write().unwrap();
            let rel = actor.bonds.entry(person.clone()).or_default();
            rel.proactive_consent = ProactiveConsent::Allowed;
            rel.channel_preference = Some("Use relay:preferred for proactive check-ins".into());
        }

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: None,
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Outreach));
                assert_eq!(
                    action.conversation,
                    Some(ConversationId("relay:preferred".into()))
                );
            }
            _ => panic!("expected outreach spawn with preferred conversation"),
        }
    }

    #[tokio::test]
    async fn proactive_intent_with_unknown_consent_drops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        set_proactive_consent(&mind, &person, ProactiveConsent::Unknown);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn dropped_one_shot_fired_intent_is_completed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        store
            .create_intent(&IntentRecord {
                id: "intent-one-shot".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Follow up once".into(),
                person: Some(PersonId("person-sam".into())),
                profile: None,
                conversation: Some(ConversationId("relay:local".into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        assert!(
            store
                .mark_intent_fired("intent-one-shot", 1000)
                .await
                .unwrap()
        );

        mind.retire_dropped_fired_intent(
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-one-shot".into(),
                task: "Follow up once".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(PersonId("person-sam".into())),
                scheduled_at: None,
                chosen_person_approved: false,
                defer_count: 0,
            }),
            &MindDecision::Drop,
        )
        .await;

        let retired = store.get_intent("intent-one-shot").await.unwrap().unwrap();
        assert_eq!(retired.status, "completed");
    }

    #[tokio::test]
    async fn dropped_recurring_fired_intent_stays_active_for_next_fire() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        store
            .create_intent(&IntentRecord {
                id: "intent-recurring".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Follow up weekly".into(),
                person: Some(PersonId("person-sam".into())),
                profile: None,
                conversation: Some(ConversationId("relay:local".into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: Some("every 2 hours".into()),
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        assert!(
            store
                .mark_intent_fired("intent-recurring", 1000)
                .await
                .unwrap()
        );

        mind.retire_dropped_fired_intent(
            &WakeEvent::IntentFired(FiredIntent {
                id: "intent-recurring".into(),
                task: "Follow up weekly".into(),
                conversation: Some(ConversationId("relay:local".into())),
                person: Some(PersonId("person-sam".into())),
                scheduled_at: None,
                chosen_person_approved: false,
                defer_count: 0,
            }),
            &MindDecision::Drop,
        )
        .await;

        let recurring = store.get_intent("intent-recurring").await.unwrap().unwrap();
        assert_eq!(recurring.status, "active");
        assert_eq!(recurring.last_fired_at, Some(1000));
        assert_eq!(recurring.fire_at, Some(8200));
    }

    #[tokio::test]
    async fn proactive_intent_with_allowed_consent_spawns_outreach() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        allow_proactive(&mind, &person);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Outreach));
                assert_eq!(
                    action.conversation,
                    Some(ConversationId("relay:local".into()))
                );
                assert_eq!(action.source_intent.as_deref(), Some("intent-1"));
            }
            _ => panic!("expected proactive outreach spawn with consent"),
        }
    }

    #[tokio::test]
    async fn proactive_intent_defers_when_gateway_is_disconnected() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind =
            test_mind_with_gateway_state(store.clone(), GatewayConnectionState::Disconnected);
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        allow_proactive(&mind, &person);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::DeferIntent(intent, delay_secs) => {
                assert_eq!(intent.defer_count, 1);
                assert_eq!(delay_secs, 5 * 60);
            }
            _ => panic!("expected proactive outreach to defer while gateway is disconnected"),
        }
    }

    #[tokio::test]
    async fn proactive_intent_drops_when_prior_proactive_outreach_is_unanswered() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        allow_proactive(&mind, &person);
        set_unanswered_proactive_outreach(&mind, &person);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn proactive_intent_with_denied_consent_drops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        set_proactive_consent(&mind, &person, ProactiveConsent::Denied);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn proactive_intent_with_unknown_conversation_person_drops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:unknown".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn proactive_intent_for_stale_conversation_drops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person);
        msg.timestamp = recent_timestamp() - 31 * 24 * 60 * 60;
        append_inbound(store.as_ref(), &msg).await;
        allow_proactive(&mind, msg.person.as_ref().unwrap());

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn proactive_intent_during_quiet_hours_defers() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let now = chrono::Utc::now();
        {
            use chrono::Timelike;
            let start_hour = now.hour() as u8;
            let end_hour = ((now.hour() + 1) % 24) as u8;
            mind.state
                .shared
                .config
                .write()
                .unwrap()
                .proactivity
                .quiet_hours_utc = Some(QuietHoursUtc {
                start_hour,
                end_hour,
            });
        }
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person);
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        allow_proactive(&mind, msg.person.as_ref().unwrap());

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::DeferIntent(intent, delay_secs) => {
                assert_eq!(intent.defer_count, 1);
                assert!((60..=3600).contains(&delay_secs));
            }
            _ => panic!("expected quiet-hours intent deferral"),
        }
    }

    #[tokio::test]
    async fn proactive_intent_without_any_target_drops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: None,
                    person: None,
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn cancelling_running_action_leaves_composing_release_to_session_guard() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mut mind = test_mind(store);
        let msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "local-msg-1",
        );
        let action = Action::respond(
            vec![msg.clone()],
            msg.conversation.clone(),
            Authority::Default,
            None,
        );
        let running_id = mind.registry.schedule(action);
        let _launch = mind.registry.launch(&running_id).expect("action launches");
        mind.gateway.acquire_composing("relay", "local").await;
        assert_eq!(mind.gateway.composing_count("relay", "local").await, 1);

        let replacement = Action::ruminate();
        mind.execute_decision(MindDecision::CancelAndSpawn(vec![running_id], replacement))
            .await;

        assert_eq!(mind.gateway.composing_count("relay", "local").await, 1);
        mind.gateway.release_composing("relay", "local").await;
        assert_eq!(mind.gateway.composing_count("relay", "local").await, 0);
    }

    #[tokio::test]
    async fn restricted_intent_does_not_spawn_outreach() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);
        let person = PersonId("restricted-person".into());
        mind.state
            .shared
            .actor
            .write()
            .unwrap()
            .set_relationship_config(&person, Some(Authority::Restricted));

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Check in".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn blocked_intent_without_chosen_person_approval_drops() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store);
        let person = PersonId("blocked-person".into());
        mind.state
            .shared
            .actor
            .write()
            .unwrap()
            .set_relationship_config(&person, Some(Authority::Blocked));

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Check in".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: false,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }

    #[tokio::test]
    async fn chosen_person_approved_restricted_intent_can_spawn_outreach() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("restricted-person".into());
        mind.state
            .shared
            .actor
            .write()
            .unwrap()
            .set_relationship_config(&person, Some(Authority::Restricted));
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Check in".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: true,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Outreach));
                assert_eq!(action.authority, Authority::Restricted);
            }
            _ => panic!("expected chosen-person-approved restricted outreach to spawn"),
        }
    }

    #[tokio::test]
    async fn chosen_person_approved_blocked_intent_can_spawn_outreach() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("blocked-person".into());
        mind.state
            .shared
            .actor
            .write()
            .unwrap()
            .set_relationship_config(&person, Some(Authority::Blocked));
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Check in".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: true,
                    defer_count: 0,
                }),
            )
            .await;

        match decision {
            MindDecision::Spawn(action) => {
                assert!(matches!(action.kind, ActionKind::Outreach));
                assert_eq!(action.authority, Authority::Blocked);
            }
            _ => panic!("expected chosen-person-approved blocked outreach to spawn"),
        }
    }

    #[tokio::test]
    async fn chosen_person_approved_intent_still_respects_denied_proactive_consent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let mind = test_mind(store.clone());
        let person = PersonId("person-sam".into());
        let mut msg = inbound(
            "relay",
            "local",
            "Sam",
            "local",
            "relay:local",
            None,
            "msg-1",
        );
        msg.person = Some(person.clone());
        msg.timestamp = recent_timestamp();
        append_inbound(store.as_ref(), &msg).await;
        set_proactive_consent(&mind, &person, ProactiveConsent::Denied);

        let decision = mind
            .build_decision(
                MindVerdict::Respond {
                    style_directive: None,
                },
                &WakeEvent::IntentFired(FiredIntent {
                    id: "intent-1".into(),
                    task: "Follow up".into(),
                    conversation: Some(ConversationId("relay:local".into())),
                    person: Some(person),
                    scheduled_at: None,
                    chosen_person_approved: true,
                    defer_count: 0,
                }),
            )
            .await;

        assert!(matches!(decision, MindDecision::Drop));
    }
}
