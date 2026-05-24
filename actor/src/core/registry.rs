use super::action::{ActionId, ActionState, ActionStatus};
use crate::store::ConversationId;
use std::collections::HashMap;

pub(crate) struct ActionRegistry {
    actions: HashMap<ActionId, ActionState>,
    max_concurrency: usize,
}

#[allow(dead_code)]
impl ActionRegistry {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            actions: HashMap::new(),
            max_concurrency,
        }
    }

    pub fn next_id(&self) -> ActionId {
        ActionId::new()
    }

    pub fn insert(&mut self, state: ActionState) {
        self.actions.insert(state.id.clone(), state);
    }

    pub fn get(&self, id: &ActionId) -> Option<&ActionState> {
        self.actions.get(id)
    }

    pub fn get_mut(&mut self, id: &ActionId) -> Option<&mut ActionState> {
        self.actions.get_mut(id)
    }

    pub fn cancel(&mut self, id: &ActionId) -> bool {
        if let Some(action) = self.actions.get_mut(id) {
            if let Some(handle) = action.handle.take() {
                handle.abort();
            }
            action.status = ActionStatus::Cancelled;
            true
        } else {
            false
        }
    }

    pub fn running_actions(&self) -> Vec<&ActionState> {
        self.actions
            .values()
            .filter(|a| matches!(a.status, ActionStatus::Running))
            .collect()
    }

    pub fn running_count(&self) -> usize {
        self.actions
            .values()
            .filter(|a| matches!(a.status, ActionStatus::Running))
            .count()
    }

    pub fn at_capacity(&self) -> bool {
        self.running_count() >= self.max_concurrency
    }

    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    pub fn for_conversation(&self, conv: &ConversationId) -> Vec<&ActionState> {
        self.actions
            .values()
            .filter(|a| {
                a.conversation.as_ref() == Some(conv)
                    && matches!(a.status, ActionStatus::Running | ActionStatus::Pending)
            })
            .collect()
    }

    pub fn mark_responded(&mut self, id: &ActionId) {
        if let Some(action) = self.actions.get_mut(id) {
            action.has_responded = true;
        }
    }

    pub fn mark_completed(&mut self, id: &ActionId) {
        if let Some(action) = self.actions.get_mut(id) {
            action.status = ActionStatus::Completed;
            action.handle = None;
        }
    }

    pub fn lowest_priority_running(&self) -> Option<&ActionState> {
        self.actions
            .values()
            .filter(|a| matches!(a.status, ActionStatus::Running))
            .min_by_key(|a| a.priority)
    }

    pub fn pending_after(&self, dependency: &ActionId) -> Vec<ActionId> {
        self.actions
            .values()
            .filter(|a| {
                matches!(a.status, ActionStatus::Pending) && a.depends_on.contains(dependency)
            })
            .map(|a| a.id.clone())
            .collect()
    }

    pub fn all_dependencies_met(&self, id: &ActionId) -> bool {
        if let Some(action) = self.actions.get(id) {
            action.depends_on.iter().all(|dep| {
                self.actions
                    .get(dep)
                    .map_or(true, |a| matches!(a.status, ActionStatus::Completed))
            })
        } else {
            false
        }
    }

    pub fn start_action(&mut self, id: &ActionId) {
        if let Some(action) = self.actions.get_mut(id) {
            action.status = ActionStatus::Running;
        }
    }

    pub fn gc(&mut self) {
        self.actions.retain(|_, a| {
            !matches!(a.status, ActionStatus::Completed | ActionStatus::Cancelled)
        });
    }
}
