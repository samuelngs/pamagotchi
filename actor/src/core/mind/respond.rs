use super::super::action::{Action, ActionKind};
use super::super::decision::MindDecision;
use super::super::event::WakeEvent;
use super::Mind;
use tracing::warn;

impl Mind {
    pub(super) fn respond_to(&self, event: &WakeEvent, style_directive: Option<String>) -> MindDecision {
        let authority = self.resolve_authority(event);

        match event {
            WakeEvent::Message(msg) => {
                if let Some(target) = self.registry.unreplied_in(&msg.conversation) {
                    let target_id = target.id.clone();
                    return MindDecision::Inject(target_id, msg.clone());
                }

                if self.registry.at_capacity() {
                    if let Some(lowest) = self.registry.lowest_priority_running() {
                        if lowest.priority < ActionKind::Respond.default_priority() {
                            let action = Action::respond(
                                vec![msg.clone()],
                                msg.conversation.clone(),
                                authority,
                                style_directive,
                            );
                            return MindDecision::CancelAndSpawn(
                                vec![lowest.id.clone()],
                                action,
                            );
                        }
                    }
                    warn!("mind wants to respond but at capacity, dropping");
                    return MindDecision::Drop;
                }

                let action = Action::respond(
                    vec![msg.clone()],
                    msg.conversation.clone(),
                    authority,
                    style_directive,
                );
                MindDecision::Spawn(action)
            }
            WakeEvent::IdleTick { .. } => {
                if self.registry.at_capacity() {
                    return MindDecision::Drop;
                }
                MindDecision::Spawn(Action::ruminate())
            }
            WakeEvent::IntentFired(intent) => {
                if self.registry.at_capacity() {
                    return MindDecision::Drop;
                }
                MindDecision::Spawn(Action::outreach(
                    intent.task.clone(),
                    intent.conversation.clone(),
                    authority,
                ))
            }
            _ => MindDecision::Drop,
        }
    }
}
