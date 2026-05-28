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
mod tests {
    use super::*;

    fn successful_outcome() -> Outcome {
        Outcome {
            responded: true,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        }
    }

    #[test]
    fn gc_retains_recent_completed_actions_for_observability() {
        let mut registry = ActionRegistry::new(1);
        let id = registry.schedule(Action::ruminate());

        registry.complete(&id, successful_outcome());
        registry.gc();

        assert!(registry.get(&id).is_some());
        assert_eq!(registry.recent_completed().len(), 1);
        assert!(matches!(
            registry.get(&id).unwrap().phase,
            Phase::Done { .. }
        ));
    }

    #[test]
    fn gc_prunes_completed_actions_after_recent_window() {
        let mut registry = ActionRegistry::new(1);
        let mut ids = Vec::new();

        for _ in 0..(MAX_RECENT_COMPLETED_ACTIONS + 2) {
            let id = registry.schedule(Action::ruminate());
            registry.complete(&id, successful_outcome());
            ids.push(id);
        }
        registry.gc();

        assert!(registry.get(&ids[0]).is_none());
        assert!(registry.get(&ids[1]).is_none());
        assert!(registry.get(ids.last().unwrap()).is_some());
        assert_eq!(
            registry.recent_completed().len(),
            MAX_RECENT_COMPLETED_ACTIONS
        );
    }

    #[test]
    fn completed_actions_do_not_count_toward_capacity() {
        let mut registry = ActionRegistry::new(1);
        let id = registry.schedule(Action::ruminate());
        registry.complete(&id, successful_outcome());

        assert!(!registry.at_capacity());
        assert!(registry.running().is_empty());
        assert_eq!(registry.recent_completed().len(), 1);
    }

    #[tokio::test]
    async fn cancel_requests_cooperative_stop_without_aborting_task() {
        let mut registry = ActionRegistry::new(1);
        let id = registry.schedule(Action::ruminate());
        let launch = registry.launch(&id).expect("action launched");
        let progress = launch.progress.clone();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            release_rx.await.ok();
            finished_tx.send(()).ok();
        });
        registry.set_handle(&id, handle);

        assert!(registry.cancel(&id));
        assert!(progress.read().unwrap().is_cancelled());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), finished_rx)
                .await
                .is_err(),
            "cancel should not hard-abort the task"
        );

        release_tx.send(()).ok();
    }

    #[test]
    fn failed_delivery_outcome_does_not_requeue_source_message() {
        let mut registry = ActionRegistry::new(1);
        let source = protocol::InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: ConversationId("relay:local".into()),
            group: None,
            identity: None,
            profile: None,
            person: None,
            content: "hello".into(),
            attachments: vec![],
            timestamp: 1,
            metadata: serde_json::Value::Null,
        };
        let id = registry.schedule(Action::respond(
            vec![source],
            ConversationId("relay:local".into()),
            crate::state::Authority::Default,
            None,
        ));

        registry.complete(
            &id,
            Outcome {
                responded: false,
                attempted_send: true,
                attempts: 1,
                ..Outcome::default()
            },
        );

        assert!(registry.follow_ups(&id).is_empty());
    }
}
