use super::action::{
    Action, ActionId, FollowUp, LaunchContext, Outcome, Phase, RunningState,
};
use protocol::ConversationId;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::info;

pub(crate) struct ActionRegistry {
    actions: HashMap<ActionId, Action>,
    max_concurrency: usize,
}

impl ActionRegistry {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            actions: HashMap::new(),
            max_concurrency,
        }
    }

    pub fn schedule(&mut self, action: Action) -> ActionId {
        let id = action.id.clone();
        self.actions.insert(id.clone(), action);
        id
    }

    pub fn launch(&mut self, id: &ActionId) -> Option<LaunchContext> {
        let action = self.actions.get_mut(id)?;
        if !action.is_queued() {
            return None;
        }

        let messages = action.source_messages.clone();
        let (tx, rx) = mpsc::channel(32);
        let progress = Arc::new(RwLock::new(RunningState::new()));
        let progress_clone = progress.clone();

        action.phase = Phase::Running {
            handle: None,
            inject_tx: tx,
            progress,
        };

        Some(LaunchContext {
            messages,
            inject_rx: rx,
            progress: progress_clone,
        })
    }

    pub fn set_handle(&mut self, id: &ActionId, handle: tokio::task::JoinHandle<()>) {
        if let Some(action) = self.actions.get_mut(id) {
            if let Phase::Running { handle: h, .. } = &mut action.phase {
                *h = Some(handle);
            }
        }
    }

    pub fn complete(&mut self, id: &ActionId, outcome: Outcome) {
        if let Some(action) = self.actions.get_mut(id) {
            action.phase = Phase::Done { outcome };
        }
    }

    pub fn cancel(&mut self, id: &ActionId) -> bool {
        if let Some(action) = self.actions.get_mut(id) {
            if let Phase::Running { handle, .. } = &mut action.phase {
                if let Some(h) = handle.take() {
                    h.abort();
                }
            }
            action.phase = Phase::Done {
                outcome: Outcome {
                    responded: false,
                    delta: None,
                    pending_messages: vec![],
                    had_injections: false,
                },
            };
            true
        } else {
            false
        }
    }

    pub fn inject(&self, id: &ActionId, msg: protocol::InboundMessage) -> bool {
        if let Some(action) = self.actions.get(id) {
            if let Phase::Running { inject_tx, .. } = &action.phase {
                return inject_tx.try_send(msg).is_ok();
            }
        }
        false
    }

    pub fn get(&self, id: &ActionId) -> Option<&Action> {
        self.actions.get(id)
    }

    pub fn running(&self) -> Vec<&Action> {
        self.actions.values().filter(|a| a.is_running()).collect()
    }

    pub fn at_capacity(&self) -> bool {
        self.actions.values().filter(|a| a.is_running()).count() >= self.max_concurrency
    }

    pub fn unreplied_in(&self, conv: &ConversationId) -> Option<&Action> {
        self.actions.values().find(|a| {
            a.is_running()
                && a.conversation.as_ref() == Some(conv)
                && !a.responded()
        })
    }

    pub fn lowest_priority_running(&self) -> Option<&Action> {
        self.actions
            .values()
            .filter(|a| a.is_running())
            .min_by_key(|a| a.priority)
    }

    pub fn ready(&self) -> Vec<ActionId> {
        self.actions
            .values()
            .filter(|a| {
                if let Phase::Queued { blocked_by } = &a.phase {
                    blocked_by.iter().all(|dep| {
                        self.actions
                            .get(dep)
                            .map_or(true, |d| matches!(d.phase, Phase::Done { .. }))
                    })
                } else {
                    false
                }
            })
            .map(|a| a.id.clone())
            .collect()
    }

    pub fn follow_ups(&self, id: &ActionId) -> Vec<FollowUp> {
        let action = match self.actions.get(id) {
            Some(a) => a,
            None => return vec![],
        };
        let outcome = match &action.phase {
            Phase::Done { outcome } => outcome,
            _ => return vec![],
        };

        let mut results = vec![];

        if !outcome.pending_messages.is_empty() {
            results.push(FollowUp::ReemitPending(outcome.pending_messages.clone()));
        }

        if action.kind.expects_response() && !outcome.responded && !outcome.had_injections {
            if !action.source_messages.is_empty() {
                info!(action = %id, "respond action failed to send message, re-queuing");
                results.push(FollowUp::Requeue(action.source_messages.clone()));
            }
        }

        results
    }

    pub fn gc(&mut self) {
        self.actions
            .retain(|_, a| !matches!(a.phase, Phase::Done { .. }));
    }
}
