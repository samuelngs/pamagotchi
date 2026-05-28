use super::super::action::{Action, ActionKind};
use super::super::decision::MindDecision;
use super::super::event::WakeEvent;
use super::{MAX_DEFER_COUNT, Mind, message_skips_injection_target};
use crate::state::{Authority, ProactiveConsent};
use crate::store::{ConversationSummary, MessageRole};
use protocol::{ConversationId, PersonId};
use tracing::warn;

const MAX_PROACTIVE_INACTIVITY_SECS: i64 = 30 * 24 * 60 * 60;
const GATEWAY_UNAVAILABLE_RETRY_SECS: u64 = 5 * 60;
const CONSOLIDATION_CAPACITY_RETRY_SECS: u64 = 5 * 60;

impl Mind {
    pub(super) async fn respond_to(
        &self,
        event: &WakeEvent,
        style_directive: Option<String>,
    ) -> MindDecision {
        let authority = self.resolve_authority(event);

        match event {
            WakeEvent::Message(msg) => {
                if let Some(target) = self.registry.unreplied_in(&msg.conversation) {
                    let target_id = target.id.clone();
                    if !message_skips_injection_target(msg, &target_id) {
                        return MindDecision::Inject(target_id, msg.clone());
                    }
                }

                if self.sender_is_typing(msg) {
                    warn!("sender is still typing, deferring response");
                    return self.defer_message_for_typing(msg, 5);
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
                            return MindDecision::CancelAndSpawn(vec![lowest.id.clone()], action);
                        }
                    }
                    warn!("mind wants to respond but at capacity, deferring message");
                    return self.defer_message(msg, 15);
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
            WakeEvent::ConsolidationDue => {
                if self.registry.at_capacity() {
                    warn!("mind wants to consolidate but is at capacity, deferring consolidation");
                    return MindDecision::DeferConsolidation(CONSOLIDATION_CAPACITY_RETRY_SECS);
                }
                MindDecision::Spawn(Action::consolidate())
            }
            WakeEvent::IntentFired(intent) => {
                let conversation = match self.outreach_conversation(intent).await {
                    Some(conversation) => conversation,
                    None => {
                        warn!(
                            intent_id = %intent.id,
                            "dropping proactive outreach because no target conversation is known"
                        );
                        return MindDecision::Drop;
                    }
                };
                let target_person = match self.outreach_target_person(intent, &conversation).await {
                    Some(person) => person,
                    None => {
                        warn!(
                            intent_id = %intent.id,
                            conversation = %conversation.0,
                            "dropping proactive outreach because target person is not verified"
                        );
                        return MindDecision::Drop;
                    }
                };
                let authority = self.authority_for_person(&target_person);
                if matches!(authority, Authority::Restricted | Authority::Blocked)
                    && !intent.chosen_person_approved
                {
                    warn!(
                        intent_id = %intent.id,
                        authority = %authority.as_str(),
                        "dropping proactive outreach for restricted authority"
                    );
                    return MindDecision::Drop;
                }
                if !self.proactive_outreach_has_consent(&target_person) {
                    if intent.chosen_person_approved
                        && !self.proactive_outreach_consent_denied(&target_person)
                    {
                        warn!(
                            intent_id = %intent.id,
                            person = %target_person.0,
                            "allowing proactive outreach without prior consent because the chosen person approved this intent"
                        );
                    } else {
                        warn!(
                            intent_id = %intent.id,
                            person = %target_person.0,
                            "dropping proactive outreach because consent is unknown or denied"
                        );
                        return MindDecision::Drop;
                    }
                }
                if self.person_has_unanswered_proactive_outreach(&target_person) {
                    warn!(
                        intent_id = %intent.id,
                        person = %target_person.0,
                        "dropping proactive outreach because prior proactive outreach is unanswered"
                    );
                    return MindDecision::Drop;
                }
                if self
                    .scheduled_outreach_obsoleted_by_inbound_reply(
                        intent,
                        &conversation,
                        &target_person,
                    )
                    .await
                {
                    warn!(
                        intent_id = %intent.id,
                        person = %target_person.0,
                        "dropping proactive outreach because target replied after it was scheduled"
                    );
                    return MindDecision::Drop;
                }
                if !self
                    .proactive_outreach_gateway_available(&conversation)
                    .await
                {
                    if intent.defer_count >= MAX_DEFER_COUNT {
                        warn!(
                            intent_id = %intent.id,
                            conversation = %conversation.0,
                            count = intent.defer_count,
                            "proactive intent gateway remained unavailable past max defer count, dropping"
                        );
                        return MindDecision::Drop;
                    }
                    let mut deferred = intent.clone();
                    deferred.defer_count += 1;
                    warn!(
                        intent_id = %intent.id,
                        conversation = %conversation.0,
                        count = deferred.defer_count,
                        "deferring proactive outreach because the gateway is unavailable"
                    );
                    return MindDecision::DeferIntent(deferred, GATEWAY_UNAVAILABLE_RETRY_SECS);
                }
                if self
                    .last_visible_message_is_assistant(Some(&conversation))
                    .await
                {
                    warn!(
                        intent_id = %intent.id,
                        "dropping proactive outreach because the last visible conversation message is already from the actor"
                    );
                    return MindDecision::Drop;
                }
                if !self.recent_user_activity_for_outreach(&conversation).await {
                    warn!(
                        intent_id = %intent.id,
                        conversation = %conversation.0,
                        "dropping proactive outreach because the conversation is stale"
                    );
                    return MindDecision::Drop;
                }
                if let Some(delay_secs) = self.proactive_quiet_hours_delay() {
                    if intent.defer_count >= MAX_DEFER_COUNT {
                        warn!(
                            intent_id = %intent.id,
                            count = intent.defer_count,
                            "proactive intent remained in quiet hours past max defer count, dropping"
                        );
                        return MindDecision::Drop;
                    }
                    let mut deferred = intent.clone();
                    deferred.defer_count += 1;
                    warn!(
                        intent_id = %intent.id,
                        delay_secs,
                        "deferring proactive outreach until quiet hours end"
                    );
                    return MindDecision::DeferIntent(deferred, delay_secs);
                }
                if self.registry.at_capacity() {
                    if intent.defer_count >= MAX_DEFER_COUNT {
                        warn!(
                            intent_id = %intent.id,
                            count = intent.defer_count,
                            "proactive intent exceeded max defer count, dropping"
                        );
                        return MindDecision::Drop;
                    }
                    let mut deferred = intent.clone();
                    deferred.defer_count += 1;
                    warn!(
                        intent_id = %intent.id,
                        count = deferred.defer_count,
                        "mind wants to run proactive intent but is at capacity, deferring"
                    );
                    return MindDecision::DeferIntent(deferred, 60);
                }
                MindDecision::Spawn(Action::outreach_with_source_intent(
                    intent.task.clone(),
                    Some(conversation),
                    authority,
                    Some(intent.id.clone()),
                ))
            }
            _ => MindDecision::Drop,
        }
    }

    fn authority_for_person(&self, person: &PersonId) -> Authority {
        let actor = self.state.read_state();
        actor
            .bonds
            .get(person)
            .map(|rel| rel.authority.clone())
            .unwrap_or(Authority::Default)
    }

    fn proactive_outreach_has_consent(&self, person: &PersonId) -> bool {
        let actor = self.state.read_state();
        let Some(rel) = actor.bonds.get(person) else {
            return false;
        };
        if rel.proactive_consent == ProactiveConsent::Denied {
            return false;
        }
        matches!(rel.authority, Authority::ChosenPerson | Authority::Trusted)
            || rel.proactive_consent == ProactiveConsent::Allowed
    }

    fn proactive_outreach_consent_denied(&self, person: &PersonId) -> bool {
        let actor = self.state.read_state();
        actor
            .bonds
            .get(person)
            .is_some_and(|rel| rel.proactive_consent == ProactiveConsent::Denied)
    }

    fn person_has_unanswered_proactive_outreach(&self, person: &PersonId) -> bool {
        let actor = self.state.read_state();
        actor.bonds.get(person).is_some_and(|rel| {
            rel.last_proactive_outbound > 0 && rel.last_proactive_outbound > rel.last_inbound
        })
    }

    async fn proactive_outreach_gateway_available(&self, conversation: &ConversationId) -> bool {
        let gateway_id = self
            .store
            .list_conversations()
            .await
            .ok()
            .and_then(|conversations| {
                conversations
                    .into_iter()
                    .find(|summary| &summary.id == conversation)
                    .and_then(|summary| summary.gateway_id)
            });
        gateway_id.is_some_and(|gateway_id| self.gateway.is_connected(&gateway_id))
    }

    async fn scheduled_outreach_obsoleted_by_inbound_reply(
        &self,
        intent: &super::super::event::FiredIntent,
        conversation: &ConversationId,
        target_person: &PersonId,
    ) -> bool {
        if intent.chosen_person_approved {
            return false;
        }
        let stored = match self.store.get_intent(&intent.id).await {
            Ok(Some(stored)) => stored,
            Ok(None) | Err(_) => return false,
        };
        if stored.kind != "scheduled" || stored.recurrence.is_some() {
            return false;
        }
        let after = intent.scheduled_at.unwrap_or(stored.created_at);
        let Ok(messages) = self.store.get_messages(conversation, 50, None).await else {
            return false;
        };
        messages.iter().rev().any(|message| {
            matches!(message.role, MessageRole::User)
                && message.timestamp > after
                && message.person.as_ref() == Some(target_person)
        })
    }

    async fn outreach_conversation(
        &self,
        intent: &super::super::event::FiredIntent,
    ) -> Option<ConversationId> {
        if let Some(conversation) = &intent.conversation {
            return Some(conversation.clone());
        }
        let person = intent.person.as_ref()?;
        let candidates = self
            .store
            .list_conversations()
            .await
            .ok()?
            .into_iter()
            .filter(|conversation| conversation.person.as_ref() == Some(person))
            .collect::<Vec<_>>();
        if let Some(preference) = self.channel_preference_for_person(person) {
            if let Some(conversation) = candidates
                .iter()
                .filter_map(|conversation| {
                    conversation_channel_preference_score(conversation, &preference)
                        .map(|score| (score, conversation))
                })
                .max_by_key(|(score, conversation)| (*score, conversation.last_message_at))
                .map(|(_, conversation)| conversation)
            {
                return Some(conversation.id.clone());
            }
        }

        candidates
            .into_iter()
            .max_by_key(|conversation| conversation.last_message_at)
            .map(|conversation| conversation.id)
    }

    fn channel_preference_for_person(&self, person: &PersonId) -> Option<String> {
        let actor = self.state.read_state();
        actor
            .bonds
            .get(person)
            .and_then(|rel| rel.channel_preference.as_deref())
            .map(str::trim)
            .filter(|preference| !preference.is_empty())
            .map(str::to_string)
    }

    async fn outreach_target_person(
        &self,
        intent: &super::super::event::FiredIntent,
        conversation: &ConversationId,
    ) -> Option<PersonId> {
        if let Some(person) = &intent.person {
            return Some(person.clone());
        }
        self.store
            .list_conversations()
            .await
            .ok()?
            .into_iter()
            .find(|summary| &summary.id == conversation)
            .and_then(|summary| summary.person)
    }

    async fn last_visible_message_is_assistant(
        &self,
        conversation: Option<&ConversationId>,
    ) -> bool {
        let Some(conversation) = conversation else {
            return false;
        };
        let Ok(messages) = self.store.get_messages(conversation, 20, None).await else {
            return false;
        };
        messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::User | MessageRole::Assistant))
            .is_some_and(|message| matches!(message.role, MessageRole::Assistant))
    }

    async fn recent_user_activity_for_outreach(&self, conversation: &ConversationId) -> bool {
        let Ok(messages) = self.store.get_messages(conversation, 50, None).await else {
            return false;
        };
        let Some(last_user_at) = messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::User))
            .map(|message| message.timestamp)
        else {
            return false;
        };
        chrono::Utc::now().timestamp().saturating_sub(last_user_at) <= MAX_PROACTIVE_INACTIVITY_SECS
    }

    fn proactive_quiet_hours_delay(&self) -> Option<u64> {
        let config = self.state.read_config();
        config
            .proactivity
            .quiet_hours_utc
            .as_ref()?
            .delay_until_end(chrono::Utc::now())
    }
}

fn conversation_channel_preference_score(
    conversation: &ConversationSummary,
    preference: &str,
) -> Option<u8> {
    let preference = preference.to_ascii_lowercase();
    if preference.contains(&conversation.id.0.to_ascii_lowercase()) {
        return Some(3);
    }
    if conversation
        .group
        .as_ref()
        .is_some_and(|group| preference.contains(&group.0.to_ascii_lowercase()))
    {
        return Some(3);
    }
    if conversation
        .gateway_id
        .as_ref()
        .is_some_and(|gateway| preference.contains(&gateway.to_ascii_lowercase()))
    {
        return Some(2);
    }
    None
}
