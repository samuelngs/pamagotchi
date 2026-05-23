use super::action::{
    ActionBrief, ActionContext, ActionId, ActionKind, ActionProgress, ActionRequest, ActionState,
    ActionStatus, ActionTiming, MindDecision,
};
use super::event::{InboundMessage, WakeEvent};
use super::registry::ActionRegistry;
use super::state::StateHandle;
use crate::identity::PersonId;
use crate::llm::Provider;
use crate::personality::Authority;
use crate::store::{ConversationId, MessageRole, Store};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct Mind {
    event_rx: mpsc::Receiver<WakeEvent>,
    event_tx: mpsc::Sender<WakeEvent>,
    registry: ActionRegistry,
    state: StateHandle,
    store: Arc<dyn Store>,
    provider: Arc<dyn Provider>,
    model: String,
}

impl Mind {
    pub fn new(
        event_rx: mpsc::Receiver<WakeEvent>,
        event_tx: mpsc::Sender<WakeEvent>,
        state: StateHandle,
        store: Arc<dyn Store>,
        provider: Arc<dyn Provider>,
        model: String,
        max_concurrency: usize,
    ) -> Self {
        Self {
            event_rx,
            event_tx,
            registry: ActionRegistry::new(max_concurrency),
            state,
            store,
            provider,
            model,
        }
    }

    pub async fn run(mut self) {
        info!("mind started");
        loop {
            match self.event_rx.recv().await {
                Some(WakeEvent::Shutdown) => {
                    self.shutdown().await;
                    break;
                }
                Some(WakeEvent::ActionCompleted { action_id, result }) => {
                    self.registry.mark_completed(&action_id);
                    self.handle_action_completed(&action_id, &result).await;
                    let decision = self.decide(WakeEvent::ActionCompleted {
                        action_id,
                        result,
                    });
                    self.execute(decision).await;
                    self.registry.gc();
                }
                Some(event) => {
                    let decision = self.decide(event);
                    self.execute(decision).await;
                    self.registry.gc();
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

    fn decide(&self, event: WakeEvent) -> MindDecision {
        if let Some(decision) = self.fast_path(&event) {
            return decision;
        }
        self.complex_decide(event)
    }

    fn fast_path(&self, event: &WakeEvent) -> Option<MindDecision> {
        match event {
            WakeEvent::Message(msg) => {
                if self.is_blocked(&msg.person) {
                    debug!(person = ?msg.person, "dropping message from blocked person");
                    return Some(MindDecision::drop());
                }

                let conv_actions = self.registry.for_conversation(&msg.conversation);

                if conv_actions.is_empty() && !self.registry.at_capacity() {
                    return Some(MindDecision::spawn_one(ActionRequest::respond(
                        vec![msg.clone()],
                        msg.conversation.clone(),
                    )));
                }

                None
            }
            WakeEvent::IdleTick { .. } => {
                if self.registry.at_capacity() {
                    return Some(MindDecision::drop());
                }
                Some(MindDecision::spawn_one(ActionRequest::ruminate()))
            }
            WakeEvent::IntentFired(intent) => {
                if self.registry.at_capacity() {
                    return Some(MindDecision::drop());
                }
                Some(MindDecision::spawn_one(ActionRequest {
                    kind: ActionKind::Respond,
                    task: intent.task.clone(),
                    conversation: intent.conversation.clone(),
                    priority: ActionKind::Outreach.default_priority(),
                    messages: vec![],
                    timing: ActionTiming::Immediate,
                    context: None,
                }))
            }
            WakeEvent::ActionCompleted { action_id, .. } => {
                let pending = self.registry.pending_after(action_id);
                if pending.is_empty() {
                    Some(MindDecision::drop())
                } else {
                    None
                }
            }
            WakeEvent::TypingUpdate { .. } => Some(MindDecision::drop()),
            WakeEvent::Shutdown => Some(MindDecision::drop()),
        }
    }

    fn complex_decide(&self, event: WakeEvent) -> MindDecision {
        match event {
            WakeEvent::Message(msg) => self.decide_message(msg),
            WakeEvent::ActionCompleted { action_id, .. } => {
                self.decide_action_completed(&action_id)
            }
            _ => MindDecision::drop(),
        }
    }

    fn decide_message(&self, msg: InboundMessage) -> MindDecision {
        let conv_actions = self.registry.for_conversation(&msg.conversation);

        let (unreplied, replied): (Vec<&ActionState>, Vec<&ActionState>) = conv_actions
            .iter()
            .filter(|a| matches!(a.status, ActionStatus::Running))
            .copied()
            .partition(|a| {
                !a.progress
                    .read()
                    .map_or(false, |p| p.responded)
            });

        if !unreplied.is_empty() {
            if let Some(target) = unreplied.first() {
                if target.inject_tx.is_some() {
                    return MindDecision::inject_one(target.id.clone(), msg);
                }
            }
            let cancel_ids: Vec<ActionId> = unreplied.iter().map(|a| a.id.clone()).collect();
            return MindDecision::cancel_and_spawn(
                cancel_ids,
                ActionRequest::respond(vec![msg.clone()], msg.conversation),
            );
        }

        if !replied.is_empty() && !self.registry.at_capacity() {
            return MindDecision::spawn_one(ActionRequest::respond(
                vec![msg.clone()],
                msg.conversation,
            ));
        }

        if self.registry.at_capacity() {
            if let Some(lowest) = self.registry.lowest_priority_running() {
                if lowest.priority < ActionKind::Respond.default_priority() {
                    return MindDecision::cancel_and_spawn(
                        vec![lowest.id.clone()],
                        ActionRequest::respond(vec![msg.clone()], msg.conversation),
                    );
                }
            }
            return MindDecision::drop();
        }

        MindDecision::spawn_one(ActionRequest::respond(vec![msg.clone()], msg.conversation))
    }

    fn decide_action_completed(&self, action_id: &ActionId) -> MindDecision {
        let pending = self.registry.pending_after(action_id);
        let mut spawn = vec![];
        for pid in &pending {
            if self.registry.all_dependencies_met(pid) {
                if let Some(action) = self.registry.get(pid) {
                    spawn.push(ActionRequest {
                        kind: action.kind.clone(),
                        task: action.task.clone(),
                        conversation: action.conversation.clone(),
                        priority: action.priority,
                        messages: vec![],
                        timing: ActionTiming::Immediate,
                        context: None,
                    });
                }
            }
        }
        if spawn.is_empty() {
            MindDecision::drop()
        } else {
            MindDecision {
                spawn,
                cancel: vec![],
                supplement: vec![],
                inject: vec![],
            }
        }
    }

    async fn gather_context(
        &self,
        conversation: Option<&ConversationId>,
        new_messages: &[InboundMessage],
        cancelled_note: Option<String>,
    ) -> ActionContext {
        let (summary, recent_messages) = if let Some(conv) = conversation {
            let summary = self
                .store
                .list_conversations()
                .await
                .ok()
                .and_then(|convs| {
                    convs
                        .into_iter()
                        .find(|c| c.id == *conv)
                        .and_then(|c| c.summary)
                });

            let recent = self
                .store
                .get_messages(conv, 20, None)
                .await
                .unwrap_or_default();

            (summary, recent)
        } else {
            (None, vec![])
        };

        let concurrent_actions: Vec<ActionBrief> = self
            .registry
            .running_actions()
            .iter()
            .map(|a| ActionBrief {
                id: a.id.clone(),
                kind: a.kind.clone(),
                task: a.task.clone(),
                conversation: a.conversation.clone(),
            })
            .collect();

        ActionContext {
            summary,
            recent_messages,
            new_messages: new_messages.to_vec(),
            cancelled_note,
            concurrent_actions,
        }
    }

    async fn handle_action_completed(
        &self,
        action_id: &ActionId,
        result: &super::action::ActionResult,
    ) {
        if let Some(ref delta) = result.delta {
            self.state.send_delta(delta.clone()).await;
            info!(%action_id, "forwarded personality delta to state task");
        }

        for msg in &result.unprocessed_messages {
            info!(%action_id, "re-queuing unprocessed message");
            self.event_tx
                .send(WakeEvent::Message(msg.clone()))
                .await
                .ok();
        }

        let action_conv = self
            .registry
            .get(action_id)
            .and_then(|a| a.conversation.clone());

        for msg in &result.injected_messages {
            if let Some(conv) = &action_conv {
                let recent = self
                    .store
                    .get_messages(conv, 5, None)
                    .await
                    .unwrap_or_default();

                let has_response_after = recent.iter().any(|m| {
                    matches!(m.role, MessageRole::Assistant) && m.timestamp > msg.timestamp
                });

                if !has_response_after {
                    info!(%action_id, "re-queuing injected message (no response found)");
                    self.event_tx
                        .send(WakeEvent::Message(msg.clone()))
                        .await
                        .ok();
                }
            } else {
                self.event_tx
                    .send(WakeEvent::Message(msg.clone()))
                    .await
                    .ok();
            }
        }
    }

    fn is_blocked(&self, person: &Option<PersonId>) -> bool {
        let Some(person) = person else {
            return false;
        };
        let personality = self.state.read_personality();
        personality
            .relationships
            .get(person)
            .map_or(false, |r| r.authority == Authority::Blocked)
    }

    async fn execute(&mut self, decision: MindDecision) {
        for id in &decision.cancel {
            if self.registry.cancel(id) {
                info!(%id, "cancelled action");
            }
        }

        for (id, msg) in decision.inject {
            if let Some(action) = self.registry.get(&id) {
                if let Some(tx) = &action.inject_tx {
                    match tx.try_send(msg) {
                        Ok(()) => info!(%id, "injected message into running action"),
                        Err(e) => warn!(%id, %e, "failed to inject message"),
                    }
                }
            }
        }

        for request in decision.spawn {
            self.spawn_action(request).await;
        }

        for (id, ctx) in &decision.supplement {
            debug!(%id, note = %ctx.note, "supplementing action");
        }
    }

    async fn spawn_action(&mut self, mut request: ActionRequest) {
        let id = self.registry.next_id();
        let depends_on = match &request.timing {
            ActionTiming::Immediate => vec![],
            ActionTiming::After(dep) => vec![dep.clone()],
            ActionTiming::AfterAll(deps) => deps.clone(),
        };

        let is_pending = !depends_on.is_empty()
            && !depends_on.iter().all(|d| {
                self.registry
                    .get(d)
                    .map_or(true, |a| matches!(a.status, ActionStatus::Completed))
            });

        let status = if is_pending {
            ActionStatus::Pending
        } else {
            ActionStatus::Running
        };

        let kind = request.kind.clone();
        let task_desc = request.task.clone();
        let conversation = request.conversation.clone();
        let priority = request.priority;

        let progress = Arc::new(RwLock::new(ActionProgress::new()));

        let state = ActionState {
            id: id.clone(),
            kind: kind.clone(),
            task: task_desc.clone(),
            conversation,
            priority,
            status,
            has_responded: false,
            depends_on,
            handle: None,
            progress: progress.clone(),
            inject_tx: None,
        };

        self.registry.insert(state);

        if !is_pending {
            if request.context.is_none() {
                let cancelled_note = request
                    .context
                    .as_ref()
                    .and_then(|c| c.cancelled_note.clone());
                request.context = Some(
                    self.gather_context(
                        request.conversation.as_ref(),
                        &request.messages,
                        cancelled_note,
                    )
                    .await,
                );
            }
            self.launch_action_task(id, kind, task_desc, request).await;
        } else {
            info!(%id, task = %task_desc, "queued pending action");
        }
    }

    async fn launch_action_task(
        &mut self,
        id: ActionId,
        kind: ActionKind,
        task_desc: String,
        request: ActionRequest,
    ) {
        let (inject_tx, inject_rx) = mpsc::channel::<InboundMessage>(32);

        if let Some(action) = self.registry.get_mut(&id) {
            action.inject_tx = Some(inject_tx);
        }

        let event_tx = self.event_tx.clone();
        let action_id = id.clone();
        let state_handle = self.state.clone();
        let store = self.store.clone();
        let provider = self.provider.clone();
        let model = self.model.clone();
        let progress = self
            .registry
            .get(&id)
            .map(|a| a.progress.clone())
            .unwrap_or_else(|| Arc::new(RwLock::new(ActionProgress::new())));

        let context = request.context;

        let handle = tokio::spawn(async move {
            info!(%action_id, kind = ?kind, task = %task_desc, "action started");

            let ctx = super::session::SessionContext {
                action_id: action_id.clone(),
                kind,
                messages: request.messages,
                conversation: request.conversation,
                state: state_handle,
                store,
                provider,
                model,
                context,
                inject_rx,
                progress,
            };

            let result = super::session::run_session(ctx).await;

            if result.delta.is_some() {
                info!(%action_id, "action produced personality delta");
            }

            event_tx
                .send(WakeEvent::ActionCompleted {
                    action_id: action_id.clone(),
                    result,
                })
                .await
                .ok();

            info!(%action_id, "action completed");
        });

        if let Some(action) = self.registry.get_mut(&id) {
            action.handle = Some(handle);
        }
    }

    async fn shutdown(&mut self) {
        info!("mind shutting down, cancelling all actions");
        let running: Vec<ActionId> = self
            .registry
            .running_actions()
            .iter()
            .map(|a| a.id.clone())
            .collect();
        for id in &running {
            self.registry.cancel(id);
        }
    }
}
