use super::*;

const TYPING_FLUSH_SCAN_LIMIT: usize = 64;

impl Mind {
    pub(super) fn update_typing(
        &mut self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        sender_external_id: &str,
        typing: bool,
    ) {
        let key = (
            conversation.clone(),
            gateway_id.to_string(),
            sender_external_id.to_string(),
        );
        let Ok(mut typing_state) = self.typing.write() else {
            warn!("typing state lock poisoned");
            return;
        };
        if typing {
            typing_state.insert(key, chrono::Utc::now().timestamp());
        } else {
            typing_state.remove(&key);
        }
    }

    pub(super) fn record_activity(&mut self) {
        self.last_activity_at = Some(Instant::now());
    }

    pub(super) fn idle_tick_is_due(&self, elapsed_secs: f64) -> bool {
        let Some(last_activity_at) = self.last_activity_at else {
            return true;
        };
        if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
            return false;
        }
        last_activity_at.elapsed().as_secs_f64() >= elapsed_secs
    }

    pub(super) fn prune_stale_typing(&self, now: i64) -> usize {
        let Ok(mut typing_state) = self.typing.write() else {
            warn!("typing state lock poisoned");
            return 0;
        };
        let before = typing_state.len();
        typing_state.retain(|_, started_at| now.saturating_sub(*started_at) <= TYPING_ACTIVE_SECS);
        before.saturating_sub(typing_state.len())
    }

    pub(super) async fn flush_deferred_typing_messages(
        &self,
        conversation: &protocol::ConversationId,
        gateway_id: &str,
        sender_external_id: &str,
    ) {
        let events = match self
            .store
            .pending_events_by_kind("message", TYPING_FLUSH_SCAN_LIMIT)
            .await
        {
            Ok(events) => events,
            Err(e) => {
                warn!(%e, "failed to scan pending message events after typing stopped");
                return;
            }
        };

        let now = chrono::Utc::now().timestamp();
        for event in events {
            let message = match serde_json::from_value::<InboundMessage>(event.payload.clone()) {
                Ok(message) => message,
                Err(e) => {
                    warn!(%e, event_id = %event.id, "failed to deserialize pending typing message");
                    let error = pending_typing_message_error(
                        event.last_error.as_deref(),
                        format!("failed to deserialize pending typing message: {e}"),
                    );
                    match self
                        .store
                        .mark_event_failed(&event.id, now, Some(&error))
                        .await
                    {
                        Ok(true) | Ok(false) => {}
                        Err(e) => {
                            warn!(%e, event_id = %event.id, "failed to mark pending typing message failed")
                        }
                    }
                    continue;
                }
            };

            if !typing_deferred_message_matches(
                &message,
                conversation,
                gateway_id,
                sender_external_id,
            ) {
                continue;
            }

            if !claim_and_send_persisted_event(
                &self.event_tx,
                self.store.as_ref(),
                &event.id,
                now,
                WakeEvent::Message(message),
                "typing flush",
            )
            .await
            {
                return;
            }
        }
    }

    pub(super) fn sender_is_typing(&self, msg: &InboundMessage) -> bool {
        let key = (
            msg.conversation.clone(),
            msg.gateway_id.clone(),
            msg.sender_external_id.clone(),
        );
        self.typing.read().ok().is_some_and(|typing_state| {
            typing_state.get(&key).is_some_and(|started_at| {
                chrono::Utc::now().timestamp() - started_at <= TYPING_ACTIVE_SECS
            })
        })
    }
}
