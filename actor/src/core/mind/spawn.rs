use super::super::action::ActionId;
use super::super::decision::MindDecision;
use super::super::event::WakeEvent;
use super::super::session::{self, SessionResult};
use super::super::tools::{SessionContext, SessionKind};
use super::Mind;
use inference::{Reasoning, RouteContext};
use tracing::{info, warn};

impl Mind {
    pub(super) async fn execute_decision(&mut self, decision: MindDecision) {
        match decision {
            MindDecision::Drop => {}
            MindDecision::Spawn(action) => {
                let id = self.registry.schedule(action);
                self.launch_action(&id).await;
            }
            MindDecision::Inject(id, msg) => {
                if !self.registry.inject(&id, msg) {
                    warn!(%id, "failed to inject message");
                }
            }
            MindDecision::CancelAndSpawn(cancel_ids, action) => {
                for cid in &cancel_ids {
                    if self.registry.cancel(cid) {
                        info!(%cid, "cancelled action");
                    }
                }
                let id = self.registry.schedule(action);
                self.launch_action(&id).await;
            }
        }
    }

    pub(super) async fn launch_action(&mut self, id: &ActionId) {
        let launch = match self.registry.launch(id) {
            Some(l) => l,
            None => return,
        };

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
        let router = self.router.clone();
        let gateway = self.gateway.clone();
        let max_turns = self.max_turns;

        let reasoning = Reasoning::Standard;
        let endpoints = self.router.resolve_chain(&RouteContext::Action(reasoning));
        let max_action_attempts = self.max_action_attempts;
        let escalate_after = self.escalate_after;

        let handle = tokio::spawn(async move {
            info!(%action_id, kind = ?kind, task = %task_desc, "action started");

            let ctx = SessionContext {
                action_id: action_id.clone(),
                kind: SessionKind::Action(kind),
                messages: launch.messages,
                conversation,
                authority,
                style_directive,
                cancelled_note,
                concurrent_summaries,
                state: state_handle,
                store,
                router,
                endpoints,
                reasoning,
                inject_rx: launch.inject_rx,
                progress: launch.progress,
                max_turns,
                max_action_attempts,
                escalate_after,
                gateway,
                session_start: std::time::Instant::now(),
            };

            let outcome = match session::run_session(ctx).await {
                SessionResult::Action(outcome) => outcome,
                SessionResult::Mind(_) => {
                    warn!(%action_id, "action session returned mind result");
                    super::super::action::Outcome {
                        responded: false,
                        delta: None,
                        pending_messages: vec![],
                        had_injections: false,
                    }
                }
            };

            if outcome.delta.is_some() {
                info!(%action_id, "action produced personality delta");
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
