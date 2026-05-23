use super::action::{
    ActionId, ActionKind, ActionRequest, ActionState, ActionStatus, ActionTiming, MindDecision,
};
use super::event::{InboundMessage, WakeEvent};
use super::registry::ActionRegistry;
use super::state::StateHandle;
use crate::identity::PersonId;
use crate::personality::Authority;
use crate::store::Store;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

pub struct Mind {
    event_rx: mpsc::Receiver<WakeEvent>,
    event_tx: mpsc::Sender<WakeEvent>,
    registry: ActionRegistry,
    state: StateHandle,
    store: Arc<dyn Store>,
}

impl Mind {
    pub fn new(
        event_rx: mpsc::Receiver<WakeEvent>,
        event_tx: mpsc::Sender<WakeEvent>,
        state: StateHandle,
        store: Arc<dyn Store>,
        max_concurrency: usize,
    ) -> Self {
        Self {
            event_rx,
            event_tx,
            registry: ActionRegistry::new(max_concurrency),
            state,
            store,
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

        let unreplied: Vec<&ActionState> = conv_actions
            .iter()
            .filter(|a| !a.has_responded && matches!(a.status, ActionStatus::Running))
            .copied()
            .collect();

        if !unreplied.is_empty() {
            let cancel_ids: Vec<ActionId> = unreplied.iter().map(|a| a.id.clone()).collect();
            let mut all_messages = vec![];
            // Original messages from cancelled actions aren't carried over —
            // the new action loads conversation history from the store
            all_messages.push(msg.clone());

            if self.registry.at_capacity() {
                return MindDecision::cancel_and_spawn(
                    cancel_ids,
                    ActionRequest::respond(all_messages, msg.conversation),
                );
            }
            return MindDecision::cancel_and_spawn(
                cancel_ids,
                ActionRequest::respond(all_messages, msg.conversation),
            );
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

        for request in decision.spawn {
            self.spawn_action(request).await;
        }

        for (id, ctx) in &decision.supplement {
            debug!(%id, note = %ctx.note, "supplementing action");
            // TODO: inject context into running action session
        }
    }

    async fn spawn_action(&mut self, request: ActionRequest) {
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
        };

        self.registry.insert(state);

        if !is_pending {
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
        _request: ActionRequest,
    ) {
        let event_tx = self.event_tx.clone();
        let action_id = id.clone();
        let state_handle = self.state.clone();
        let store = self.store.clone();

        let handle = tokio::spawn(async move {
            info!(%action_id, kind = ?kind, task = %task_desc, "action started");

            // TODO: full action session — LLM call, MCP tools, reflection
            // For now, stub: just log and complete
            let result = run_action_stub(&action_id, &kind, state_handle, store).await;

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

async fn run_action_stub(
    _id: &ActionId,
    _kind: &ActionKind,
    _state: StateHandle,
    _store: Arc<dyn Store>,
) -> super::action::ActionResult {
    super::action::ActionResult {
        delta: None,
        thoughts: vec![],
        memories_formed: vec![],
    }
}
