use super::super::action::ActionId;
use super::super::event::WakeEvent;
use super::Mind;
use crate::store::MessageRole;
use tracing::info;

impl Mind {
    pub(super) async fn handle_action_completed(
        &self,
        action_id: &ActionId,
        result: &super::super::action::ActionResult,
    ) {
        if let Some(ref delta) = result.delta {
            self.state.send_delta(delta.clone()).await;
            info!(%action_id, "forwarded personality delta to state task");
        }

        for msg in &result.unprocessed_messages {
            info!(%action_id, "re-queuing unprocessed message");
            self.event_tx
                .send(WakeEvent::Message(msg.clone()))
                .await
                .ok();
        }

        let action_conv = self
            .registry
            .get(action_id)
            .and_then(|a| a.conversation.clone());

        for msg in &result.injected_messages {
            if let Some(conv) = &action_conv {
                let recent = self
                    .store
                    .get_messages(conv, 5, None)
                    .await
                    .unwrap_or_default();

                let has_response_after = recent.iter().any(|m| {
                    matches!(m.role, MessageRole::Assistant) && m.timestamp > msg.timestamp
                });

                if !has_response_after {
                    info!(%action_id, "re-queuing injected message (no response found)");
                    self.event_tx
                        .send(WakeEvent::Message(msg.clone()))
                        .await
                        .ok();
                }
            } else {
                self.event_tx
                    .send(WakeEvent::Message(msg.clone()))
                    .await
                    .ok();
            }
        }
    }
}

pub fn describe(event: &WakeEvent) -> String {
    match event {
        WakeEvent::Message(msg) => {
            format!(
                "New message in conversation {}:\n{}",
                msg.conversation.0,
                msg.display_content()
            )
        }
        WakeEvent::IdleTick { elapsed_secs } => {
            format!("Idle tick. {:.0} seconds since last activity.", elapsed_secs)
        }
        WakeEvent::IntentFired(intent) => {
            let conv = intent
                .conversation
                .as_ref()
                .map_or("none".to_string(), |c| c.0.clone());
            format!(
                "Scheduled intent fired: {} (conversation: {})",
                intent.task, conv
            )
        }
        WakeEvent::ActionCompleted { action_id, result } => {
            let has_delta = result.delta.is_some();
            let unprocessed = result.unprocessed_messages.len();
            format!(
                "Action {} completed. personality_delta={} unprocessed_messages={}",
                action_id, has_delta, unprocessed
            )
        }
        WakeEvent::TypingUpdate {
            person, typing, ..
        } => {
            format!(
                "{} {} typing.",
                person.0,
                if *typing { "started" } else { "stopped" }
            )
        }
        WakeEvent::Shutdown => unreachable!(),
    }
}
