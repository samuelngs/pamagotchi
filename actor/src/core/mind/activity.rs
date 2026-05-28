use super::*;

pub(super) fn event_counts_as_activity(event: &WakeEvent) -> bool {
    matches!(
        event,
        WakeEvent::ActionCompleted { .. }
            | WakeEvent::IntentFired(_)
            | WakeEvent::TypingUpdate { .. }
            | WakeEvent::MessageEdited { .. }
            | WakeEvent::MessageDeleted { .. }
    )
}
