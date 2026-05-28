use super::action::{Action, ActionId, FollowUp, LaunchContext, Outcome, Phase, RunningState};
use protocol::{ConversationId, InboundMessage};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::info;

const MAX_RECENT_COMPLETED_ACTIONS: usize = 32;

pub(crate) struct ActionRegistry {
    actions: HashMap<ActionId, Action>,
    recent_completed: VecDeque<ActionId>,
    max_concurrency: usize,
}

impl ActionRegistry {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            actions: HashMap::new(),
            recent_completed: VecDeque::new(),
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

    pub fn complete(&mut self, id: &ActionId, outcome: Outcome) -> bool {
        if let Some(action) = self.actions.get_mut(id) {
            if matches!(action.phase, Phase::Done { .. }) {
                return false;
            }
            append_review_messages(&mut action.source_messages, &outcome.review_messages);
            action.phase = Phase::Done { outcome };
            self.remember_completed(id.clone());
            true
        } else {
            false
        }
    }

    pub fn cancel(&mut self, id: &ActionId) -> bool {
        if let Some(action) = self.actions.get_mut(id) {
            if let Phase::Running {
                handle, progress, ..
            } = &mut action.phase
            {
                if let Ok(progress) = progress.read() {
                    progress.request_cancel();
                } else if let Some(h) = handle.take() {
                    h.abort();
                }
            }
            action.phase = Phase::Done {
                outcome: Outcome {
                    cancelled: true,
                    ..Outcome::default()
                },
            };
            self.remember_completed(id.clone());
            true
        } else {
            false
        }
    }

    pub fn injection_sender(
        &self,
        id: &ActionId,
    ) -> Option<mpsc::Sender<protocol::InboundMessage>> {
        if let Some(action) = self.actions.get(id) {
            if let Phase::Running { inject_tx, .. } = &action.phase {
                return Some(inject_tx.clone());
            }
        }
        None
    }

    pub fn get(&self, id: &ActionId) -> Option<&Action> {
        self.actions.get(id)
    }

    pub fn running(&self) -> Vec<&Action> {
        self.actions.values().filter(|a| a.is_running()).collect()
    }

    pub fn queued_len(&self) -> usize {
        self.actions.values().filter(|a| a.is_queued()).count()
    }

    pub fn running_len(&self) -> usize {
        self.actions.values().filter(|a| a.is_running()).count()
    }

    pub fn retained_completed_len(&self) -> usize {
        self.recent_completed.len()
    }

    pub fn recent_completed(&self) -> Vec<&Action> {
        self.recent_completed
            .iter()
            .filter_map(|id| self.actions.get(id))
            .filter(|action| matches!(action.phase, Phase::Done { .. }))
            .collect()
    }

    pub fn at_capacity(&self) -> bool {
        self.actions.values().filter(|a| a.is_running()).count() >= self.max_concurrency
    }

    pub fn unreplied_in(&self, conv: &ConversationId) -> Option<&Action> {
        self.actions
            .values()
            .find(|a| a.is_running() && a.conversation.as_ref() == Some(conv) && !a.responded())
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

        if action.kind.expects_response()
            && !outcome.responded
            && !outcome.attempted_send
            && !outcome.had_injections
            && !outcome.cancelled
        {
            if !action.source_messages.is_empty() {
                info!(action = %id, "respond action ended without attempting send_message, re-queuing");
                results.push(FollowUp::Requeue(action.source_messages.clone()));
            }
        }

        results
    }

    pub fn gc(&mut self) {
        while self.recent_completed.len() > MAX_RECENT_COMPLETED_ACTIONS {
            self.recent_completed.pop_front();
        }
        let retained_completed = self
            .recent_completed
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        self.actions.retain(|id, action| {
            !matches!(action.phase, Phase::Done { .. }) || retained_completed.contains(id)
        });
    }

    fn remember_completed(&mut self, id: ActionId) {
        self.recent_completed.retain(|existing| existing != &id);
        self.recent_completed.push_back(id);
    }
}

fn append_review_messages(
    source_messages: &mut Vec<InboundMessage>,
    review_messages: &[InboundMessage],
) {
    for message in review_messages {
        let duplicate = source_messages.iter().any(|existing| {
            existing.gateway_id == message.gateway_id && existing.message_id == message.message_id
        });
        if !duplicate {
            source_messages.push(message.clone());
        }
    }
}

#[cfg(test)]
mod tests;
