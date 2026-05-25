use super::super::action::{
    ActionBrief, ActionContext, ActionId, ActionKind, ActionProgress,
    ActionRequest, ActionState, ActionStatus, ActionTiming,
};
use super::super::decision::MindDecision;
use super::super::event::WakeEvent;
use protocol::InboundMessage;
use super::super::session::{self, SessionResult};
use super::super::tools::{SessionContext, SessionKind};
use super::Mind;
use inference::{Reasoning, RouteContext};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{info, warn};

impl Mind {
    pub(super) async fn execute_decision(&mut self, decision: MindDecision) {
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
            tracing::debug!(%id, note = %ctx.note, "supplementing action");
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
                request.context = Some(self.gather_context(None));
            }
            self.launch_action(id, kind, task_desc, request).await;
        } else {
            info!(%id, task = %task_desc, "queued pending action");
        }
    }

    async fn launch_action(
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
        let router = self.router.clone();
        let gateway = self.gateway.clone();
        let max_turns = self.max_turns;
        let progress = self
            .registry
            .get(&id)
            .map(|a| a.progress.clone())
            .unwrap_or_else(|| Arc::new(RwLock::new(ActionProgress::new())));

        let endpoints = self.router.resolve_chain(&RouteContext::Action(Reasoning::Standard));
        let context = request.context;

        let handle = tokio::spawn(async move {
            info!(%action_id, kind = ?kind, task = %task_desc, "action started");

            let ctx = SessionContext {
                action_id: action_id.clone(),
                kind: SessionKind::Action(kind),
                messages: request.messages,
                conversation: request.conversation,
                authority: request.authority,
                state: state_handle,
                store,
                router,
                endpoints,
                context,
                inject_rx,
                progress,
                max_turns,
                gateway,
                session_start: std::time::Instant::now(),
            };

            let result = match session::run_session(ctx).await {
                SessionResult::Action(result) => result,
                SessionResult::Mind(_) => {
                    warn!(%action_id, "action session returned mind result");
                    super::super::action::ActionResult {
                        delta: None,
                        thoughts: vec![],
                        memories_formed: vec![],
                        unprocessed_messages: vec![],
                        injected_messages: vec![],
                    }
                }
            };

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

    pub(super) fn gather_context(&self, cancelled_note: Option<String>) -> ActionContext {
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
            cancelled_note,
            concurrent_actions,
        }
    }
}
