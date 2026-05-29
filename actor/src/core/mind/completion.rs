use super::super::lifecycle::ActorLifecycleEvent;
use super::*;

impl Mind {
    pub(super) async fn retire_dropped_fired_intent(
        &self,
        event: &WakeEvent,
        decision: &MindDecision,
    ) {
        if !matches!(decision, MindDecision::Drop) {
            return;
        }
        let WakeEvent::IntentFired(intent) = event else {
            return;
        };

        let stored = match self.store.get_intent(&intent.id).await {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                warn!(
                    intent_id = %intent.id,
                    "dropped fired intent was missing from store"
                );
                return;
            }
            Err(e) => {
                warn!(
                    %e,
                    intent_id = %intent.id,
                    "failed to load dropped fired intent"
                );
                return;
            }
        };
        if stored.status != "fired" {
            return;
        }

        let now = chrono::Utc::now().timestamp();
        match self.store.complete_intent(&intent.id, now).await {
            Ok(true) => info!(
                intent_id = %intent.id,
                "retired dropped fired intent"
            ),
            Ok(false) => {}
            Err(e) => warn!(
                %e,
                intent_id = %intent.id,
                "failed to retire dropped fired intent"
            ),
        }
    }

    pub(super) async fn handle_completed(
        &mut self,
        action_id: &ActionId,
        outcome: crate::core::action::Outcome,
    ) {
        if !self.registry.complete(action_id, outcome) {
            debug!(%action_id, "ignoring completion for action that is no longer running");
            return;
        }
        if let Some(action) = self.registry.get(action_id) {
            let failed = action.kind.expects_response() && !action.responded();
            self.metrics.record_action_completed(failed);
        }
        self.refresh_registry_metrics();

        if let Some(action) = self.registry.get(action_id) {
            if let Some(event) = ActorLifecycleEvent::action_completed(action) {
                self.emit_lifecycle(event);
            }
        }

        if let Some(action) = self.registry.get(action_id) {
            if let crate::core::action::Phase::Done { outcome } = &action.phase {
                if let Some(ref delta) = outcome.delta {
                    self.state.send_delta(delta.clone()).await;
                    info!(%action_id, "forwarded personality delta");
                }
            }
        }

        self.complete_successful_outreach_source_intent(action_id)
            .await;

        if let Some(review) = self.build_post_turn_review(action_id) {
            let review_action_id = review.id.clone();
            match self
                .store
                .mark_review_scheduled(
                    &action_id.0,
                    &review_action_id.0,
                    chrono::Utc::now().timestamp(),
                )
                .await
            {
                Ok(true) => {
                    self.reviewed_actions.insert(action_id.clone());
                    let review_id = self.schedule_action(review);
                    info!(%action_id, %review_id, "scheduled post-turn review");
                }
                Ok(false) => {
                    self.reviewed_actions.insert(action_id.clone());
                    info!(%action_id, "post-turn review already scheduled");
                }
                Err(e) => {
                    warn!(%action_id, %e, "failed to persist review watermark; skipping review scheduling");
                }
            }
        }

        self.retire_handled_triggered_intents(action_id).await;

        let follow_ups = self.registry.follow_ups(action_id);
        for fu in follow_ups {
            match fu {
                FollowUp::Requeue(msgs) => {
                    for msg in msgs {
                        info!(%action_id, "re-queuing source message after failed respond");
                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                }
                FollowUp::ReemitPending(msgs) => {
                    for msg in msgs {
                        info!(%action_id, "re-emitting pending message");
                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                }
            }
        }

        let ready = self.registry.ready();
        for id in ready {
            self.launch_action(&id).await;
        }

        self.registry.gc();
        self.refresh_registry_metrics();
        debug!(
            recent_completed = self.registry.recent_completed().len(),
            "action registry garbage collection complete"
        );
    }

    pub(super) async fn complete_successful_outreach_source_intent(&self, action_id: &ActionId) {
        let Some(action) = self.registry.get(action_id) else {
            return;
        };
        if !matches!(action.kind, ActionKind::Outreach) {
            return;
        }
        let crate::core::action::Phase::Done { outcome } = &action.phase else {
            return;
        };
        if !outcome.responded {
            return;
        }
        let Some(intent_id) = action.source_intent.as_deref() else {
            return;
        };

        let intent = match self.store.get_intent(intent_id).await {
            Ok(Some(intent)) => intent,
            Ok(None) => {
                warn!(
                    %action_id,
                    intent_id,
                    "outreach source intent was missing after successful action"
                );
                return;
            }
            Err(e) => {
                warn!(
                    %e,
                    %action_id,
                    intent_id,
                    "failed to load outreach source intent after successful action"
                );
                return;
            }
        };
        if intent.status != "fired" {
            return;
        }

        let now = chrono::Utc::now().timestamp();
        match self.store.complete_intent(intent_id, now).await {
            Ok(true) => info!(
                %action_id,
                intent_id,
                "marked successful outreach source intent completed"
            ),
            Ok(false) => {}
            Err(e) => warn!(
                %e,
                %action_id,
                intent_id,
                "failed to mark successful outreach source intent completed"
            ),
        }
    }

    pub(super) fn build_post_turn_review(&mut self, action_id: &ActionId) -> Option<Action> {
        let action = self.registry.get(action_id)?;
        let crate::core::action::Phase::Done { outcome } = &action.phase else {
            return None;
        };
        if !outcome.responded {
            return None;
        }
        if !matches!(action.kind, ActionKind::Respond | ActionKind::Outreach) {
            return None;
        }
        if self.reviewed_actions.contains(action_id) {
            return None;
        }

        Some(Action::review(
            action_id.clone(),
            action.source_messages.clone(),
            action.conversation.clone(),
            action.relationship_standing.clone(),
        ))
    }

    pub(super) async fn retire_handled_triggered_intents(&self, action_id: &ActionId) {
        let Some(action) = self.registry.get(action_id) else {
            return;
        };
        if !matches!(action.kind, ActionKind::Respond) {
            return;
        }
        let crate::core::action::Phase::Done { outcome } = &action.phase else {
            return;
        };
        if !outcome.responded {
            return;
        }
        let Some(message) = action.source_messages.first() else {
            return;
        };

        let now = chrono::Utc::now().timestamp();
        let intents = match self
            .store
            .active_intents_for_context(
                message.person.as_ref(),
                message.profile.as_ref(),
                Some(&message.conversation),
                now,
                5,
            )
            .await
        {
            Ok(intents) => intents,
            Err(e) => {
                warn!(%e, %action_id, "failed to load triggered intents after response");
                return;
            }
        };

        for intent in intents {
            if !triggered_intent_satisfied_by_inbound_response(&intent, message) {
                continue;
            }
            match self.store.complete_intent(&intent.id, now).await {
                Ok(true) => info!(
                    %action_id,
                    intent_id = %intent.id,
                    "marked handled triggered intent completed after response"
                ),
                Ok(false) => {}
                Err(e) => warn!(
                    %e,
                    %action_id,
                    intent_id = %intent.id,
                    "failed to mark handled triggered intent completed"
                ),
            }
        }
    }
}
