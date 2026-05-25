use super::super::action::{
    ActionId, ActionKind, ActionRequest, ActionState, ActionStatus, ActionTiming,
};
use super::super::decision::MindDecision;
use super::super::event::WakeEvent;
use super::Mind;
use crate::state::Authority;
use tracing::warn;

impl Mind {
    pub(super) fn respond_to(&self, event: &WakeEvent) -> MindDecision {
        let authority = self.resolve_authority(event);

        match event {
            WakeEvent::Message(msg) => {
                let conv_actions = self.registry.for_conversation(&msg.conversation);
                let running: Vec<&ActionState> = conv_actions
                    .iter()
                    .filter(|a| matches!(a.status, ActionStatus::Running))
                    .copied()
                    .collect();

                let unreplied: Vec<&ActionState> = running
                    .iter()
                    .filter(|a| !a.progress.read().map_or(false, |p| p.responded))
                    .copied()
                    .collect();

                if !unreplied.is_empty() {
                    if let Some(target) = unreplied.first() {
                        if target.inject_tx.is_some() {
                            return MindDecision::inject_one(target.id.clone(), msg.clone());
                        }
                    }
                    let cancel_ids: Vec<ActionId> =
                        unreplied.iter().map(|a| a.id.clone()).collect();
                    return MindDecision::cancel_and_spawn(
                        cancel_ids,
                        ActionRequest::respond(
                            vec![msg.clone()],
                            msg.conversation.clone(),
                            authority,
                        ),
                    );
                }

                if self.registry.at_capacity() {
                    if let Some(lowest) = self.registry.lowest_priority_running() {
                        if lowest.priority < ActionKind::Respond.default_priority() {
                            return MindDecision::cancel_and_spawn(
                                vec![lowest.id.clone()],
                                ActionRequest::respond(
                                    vec![msg.clone()],
                                    msg.conversation.clone(),
                                    authority,
                                ),
                            );
                        }
                    }
                    warn!("mind wants to respond but at capacity, dropping");
                    return MindDecision::drop();
                }

                MindDecision::spawn_one(ActionRequest::respond(
                    vec![msg.clone()],
                    msg.conversation.clone(),
                    authority,
                ))
            }
            WakeEvent::IdleTick { .. } => {
                if self.registry.at_capacity() {
                    return MindDecision::drop();
                }
                MindDecision::spawn_one(ActionRequest::ruminate())
            }
            WakeEvent::IntentFired(intent) => {
                if self.registry.at_capacity() {
                    return MindDecision::drop();
                }
                MindDecision::spawn_one(ActionRequest {
                    kind: ActionKind::Respond,
                    task: intent.task.clone(),
                    conversation: intent.conversation.clone(),
                    priority: ActionKind::Outreach.default_priority(),
                    messages: vec![],
                    timing: ActionTiming::Immediate,
                    context: None,
                    authority,
                })
            }
            WakeEvent::ActionCompleted { action_id, result: _ } => {
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
                                authority: Authority::Default,
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
            WakeEvent::TypingUpdate { .. } => MindDecision::drop(),
            WakeEvent::Shutdown => MindDecision::drop(),
        }
    }
}
