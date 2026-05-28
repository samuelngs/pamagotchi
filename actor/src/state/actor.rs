use super::{
    AffectState, Authority, Belief, CoreTraits, Delta, GrowthConfig, Interest, ProactiveConsent,
    Relationship, RelationshipInteraction,
};
use protocol::PersonId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorState {
    pub created_at: i64,
    pub traits: CoreTraits,
    pub beliefs: Vec<Belief>,
    pub bonds: HashMap<PersonId, Relationship>,
    pub interests: Vec<Interest>,
    pub affect: AffectState,
    pub growth_log: Vec<GrowthEvent>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GrowthEvent {
    pub timestamp: i64,
    pub description: String,
    pub person: Option<PersonId>,
}

impl ActorState {
    pub fn new(traits: CoreTraits) -> Self {
        Self {
            created_at: now(),
            traits,
            beliefs: vec![],
            bonds: HashMap::new(),
            interests: vec![],
            affect: AffectState::default(),
            growth_log: vec![],
        }
    }

    pub fn apply_delta(&mut self, delta: &Delta, config: &GrowthConfig) {
        let rate = config.rate_multiplier();
        let damping = 1.0 - config.core_trait_inertia;

        for nudge in &delta.trait_nudges {
            self.traits.nudge(
                &nudge.trait_name,
                nudge.direction * damping * rate,
                &config.trait_floors,
            );
        }

        for change in &delta.belief_changes {
            self.apply_belief_change(change, rate);
        }

        for change in &delta.relationship_changes {
            self.apply_relationship_change(change, rate);
        }

        for update in &delta.relationship_signal_updates {
            self.apply_relationship_signal_update(update, rate);
        }

        for topic in &delta.new_interests {
            self.strengthen_interest(topic, rate, delta.triggered_by.as_ref());
        }

        self.affect.shift(&delta.affect_shift);

        if let Some(ref note) = delta.growth_note {
            self.growth_log.push(GrowthEvent {
                timestamp: now(),
                description: note.clone(),
                person: delta.triggered_by.clone(),
            });
            if self.growth_log.len() > 1000 {
                self.growth_log.drain(0..self.growth_log.len() - 1000);
            }
        }
    }

    pub fn tick_idle(&mut self, elapsed_secs: f64) {
        self.affect.mean_revert(elapsed_secs);

        for interest in &mut self.interests {
            interest.decay(elapsed_secs);
        }
        self.interests.retain(|i| i.intensity > 0.01);

        for rel in self.bonds.values_mut() {
            let decay = (-0.00001 * elapsed_secs).exp() as f32;
            rel.emotional_valence *= decay;
        }
    }

    fn apply_belief_change(&mut self, change: &super::BeliefChange, rate: f32) {
        if let Some(belief) = self
            .beliefs
            .iter_mut()
            .find(|b| b.topic == change.topic && b.about == change.about)
        {
            if let Some(ref stance) = change.new_stance {
                belief.stance = stance.clone();
            }
            belief.confidence =
                (belief.confidence + change.confidence_delta * rate).clamp(0.0, 1.0);
            belief.last_challenged = Some(now());
        } else if let Some(ref stance) = change.new_stance {
            self.beliefs.push(Belief {
                topic: change.topic.clone(),
                stance: stance.clone(),
                confidence: (0.5 + change.confidence_delta * rate).clamp(0.0, 1.0),
                formed_at: now(),
                last_challenged: None,
                about: change.about.clone(),
            });
        }
    }

    fn apply_relationship_change(&mut self, change: &super::RelationshipChange, rate: f32) {
        let person = change.person.clone();
        let rel = self
            .bonds
            .entry(person)
            .or_insert_with(Relationship::default);
        rel.trust = (rel.trust + change.trust_delta * rate).clamp(
            0.0,
            change
                .trust_ceiling
                .unwrap_or_else(|| rel.authority.trust_ceiling())
                .min(rel.authority.trust_ceiling()),
        );
        rel.familiarity = (rel.familiarity + change.familiarity_delta * rate).clamp(0.0, 1.0);
        rel.emotional_valence =
            (rel.emotional_valence + change.valence_delta * rate).clamp(-1.0, 1.0);
        if let Some(consent) = &change.proactive_consent {
            rel.proactive_consent = consent.clone();
        }
        if let Some(response_cadence) =
            normalize_relationship_preference(change.response_cadence.as_deref())
        {
            rel.response_cadence = Some(response_cadence);
        }
        if let Some(channel_preference) =
            normalize_relationship_preference(change.channel_preference.as_deref())
        {
            rel.channel_preference = Some(channel_preference);
        }
        if let Some(interaction) = change.interaction {
            let now = now();
            rel.last_interaction = now;
            rel.interaction_count = rel.interaction_count.saturating_add(1);
            match interaction {
                RelationshipInteraction::Inbound => {
                    rel.last_inbound = now;
                    rel.inbound_count = rel.inbound_count.saturating_add(1);
                }
                RelationshipInteraction::Outbound => {
                    rel.last_outbound = now;
                    rel.outbound_count = rel.outbound_count.saturating_add(1);
                }
                RelationshipInteraction::ProactiveOutbound => {
                    rel.last_outbound = now;
                    rel.outbound_count = rel.outbound_count.saturating_add(1);
                    rel.last_proactive_outbound = now;
                    rel.proactive_outbound_count = rel.proactive_outbound_count.saturating_add(1);
                }
            }
        }
    }

    fn apply_relationship_signal_update(
        &mut self,
        update: &super::RelationshipSignalUpdate,
        rate: f32,
    ) {
        let rel = self
            .bonds
            .entry(update.person.clone())
            .or_insert_with(Relationship::default);
        rel.closeness = (rel.closeness + update.closeness_delta * rate).clamp(0.0, 1.0);
        rel.reliability = (rel.reliability + update.reliability_delta * rate).clamp(0.0, 1.0);
        rel.reciprocity = (rel.reciprocity + update.reciprocity_delta * rate).clamp(0.0, 1.0);
        rel.conflict_level = (rel.conflict_level + update.conflict_delta * rate).clamp(0.0, 1.0);
    }

    pub fn set_relationship_config(
        &mut self,
        person: &PersonId,
        authority: Option<super::Authority>,
    ) {
        let rel = self
            .bonds
            .entry(person.clone())
            .or_insert_with(Relationship::default);
        if let Some(a) = authority {
            rel.authority = a;
            rel.trust = rel.trust.min(rel.authority.trust_ceiling());
        }
    }

    pub fn merge_person_context(&mut self, from: &PersonId, into: &PersonId) {
        if from == into {
            return;
        }
        let Some(from_rel) = self.bonds.remove(from) else {
            return;
        };
        let into_rel = self
            .bonds
            .entry(into.clone())
            .or_insert_with(Relationship::default);

        into_rel.authority = merge_authority(&into_rel.authority, &from_rel.authority);
        into_rel.trust = into_rel
            .trust
            .max(from_rel.trust)
            .min(into_rel.authority.trust_ceiling());
        into_rel.familiarity = into_rel
            .familiarity
            .max(from_rel.familiarity)
            .clamp(0.0, 1.0);
        into_rel.emotional_valence = merge_valence(into_rel, &from_rel);
        into_rel.proactive_consent =
            merge_proactive_consent(&into_rel.proactive_consent, &from_rel.proactive_consent);
        into_rel.last_interaction = into_rel.last_interaction.max(from_rel.last_interaction);
        into_rel.interaction_count = into_rel
            .interaction_count
            .saturating_add(from_rel.interaction_count);
        into_rel.last_inbound = into_rel.last_inbound.max(from_rel.last_inbound);
        into_rel.last_outbound = into_rel.last_outbound.max(from_rel.last_outbound);
        into_rel.inbound_count = into_rel
            .inbound_count
            .saturating_add(from_rel.inbound_count);
        into_rel.outbound_count = into_rel
            .outbound_count
            .saturating_add(from_rel.outbound_count);
        into_rel.last_proactive_outbound = into_rel
            .last_proactive_outbound
            .max(from_rel.last_proactive_outbound);
        into_rel.proactive_outbound_count = into_rel
            .proactive_outbound_count
            .saturating_add(from_rel.proactive_outbound_count);
        into_rel.closeness = into_rel.closeness.max(from_rel.closeness).clamp(0.0, 1.0);
        into_rel.reliability = into_rel
            .reliability
            .max(from_rel.reliability)
            .clamp(0.0, 1.0);
        into_rel.reciprocity = into_rel
            .reciprocity
            .max(from_rel.reciprocity)
            .clamp(0.0, 1.0);
        into_rel.conflict_level = into_rel
            .conflict_level
            .max(from_rel.conflict_level)
            .clamp(0.0, 1.0);
        if into_rel.response_cadence.is_none() {
            into_rel.response_cadence = from_rel.response_cadence;
        }
        if into_rel.channel_preference.is_none() {
            into_rel.channel_preference = from_rel.channel_preference;
        }
    }

    fn strengthen_interest(&mut self, topic: &str, rate: f32, triggered_by: Option<&PersonId>) {
        if let Some(interest) = self.interests.iter_mut().find(|i| i.topic == topic) {
            interest.intensity = (interest.intensity + 0.1 * rate).clamp(0.0, 1.0);
            interest.last_engaged = now();
        } else {
            self.interests.push(Interest {
                topic: topic.to_string(),
                intensity: 0.3 * rate,
                origin: String::new(),
                origin_person: triggered_by.cloned(),
                last_engaged: now(),
            });
        }
    }
}

fn merge_authority(a: &Authority, b: &Authority) -> Authority {
    use Authority::*;
    if matches!(a, Blocked) || matches!(b, Blocked) {
        Blocked
    } else if matches!(a, Restricted) || matches!(b, Restricted) {
        Restricted
    } else if matches!(a, Owner) || matches!(b, Owner) {
        Owner
    } else if matches!(a, Trusted) || matches!(b, Trusted) {
        Trusted
    } else {
        Default
    }
}

fn merge_valence(into: &Relationship, from: &Relationship) -> f32 {
    let total = into
        .interaction_count
        .saturating_add(from.interaction_count);
    if total == 0 {
        return ((into.emotional_valence + from.emotional_valence) / 2.0).clamp(-1.0, 1.0);
    }
    let weighted = into.emotional_valence * into.interaction_count as f32
        + from.emotional_valence * from.interaction_count as f32;
    (weighted / total as f32).clamp(-1.0, 1.0)
}

fn merge_proactive_consent(a: &ProactiveConsent, b: &ProactiveConsent) -> ProactiveConsent {
    use ProactiveConsent::*;
    if matches!(a, Denied) || matches!(b, Denied) {
        Denied
    } else if matches!(a, Allowed) || matches!(b, Allowed) {
        Allowed
    } else {
        Unknown
    }
}

fn normalize_relationship_preference(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(240).collect())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        Authority, GrowthConfig, ProactiveConsent, RelationshipChange, RelationshipInteraction,
        RelationshipSignalUpdate,
    };

    #[test]
    fn default_relationship_trust_is_capped() {
        let person = PersonId("person-default".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 10.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        assert_eq!(state.bonds[&person].authority, Authority::Default);
        assert_eq!(
            state.bonds[&person].trust,
            Authority::Default.trust_ceiling()
        );
    }

    #[test]
    fn lowering_authority_lowers_existing_trust_ceiling() {
        let person = PersonId("person-restricted".into());
        let mut state = ActorState::new(CoreTraits::default());
        state.set_relationship_config(&person, Some(Authority::Owner));
        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 10.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        state.set_relationship_config(&person, Some(Authority::Restricted));

        assert_eq!(
            state.bonds[&person].trust,
            Authority::Restricted.trust_ceiling()
        );
    }

    #[test]
    fn relationship_delta_trust_ceiling_blocks_accumulation() {
        let person = PersonId("person-stranger".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.1,
                    trust_ceiling: Some(0.3),
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        assert_eq!(state.bonds[&person].trust, 0.3);
    }

    #[test]
    fn relationship_delta_updates_proactive_consent() {
        let person = PersonId("person-consent".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: Some(ProactiveConsent::Allowed),
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );
        assert_eq!(
            state.bonds[&person].proactive_consent,
            ProactiveConsent::Allowed
        );
        assert_eq!(state.bonds[&person].interaction_count, 0);

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: Some(ProactiveConsent::Denied),
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );
        assert_eq!(
            state.bonds[&person].proactive_consent,
            ProactiveConsent::Denied
        );
    }

    #[test]
    fn relationship_signal_updates_are_bounded_scalar_state() {
        let person = PersonId("person-signals".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_signal_updates: vec![RelationshipSignalUpdate {
                    person: person.clone(),
                    closeness_delta: 0.4,
                    reliability_delta: 2.0,
                    reciprocity_delta: 0.3,
                    conflict_delta: -0.5,
                    reason: "reviewed repeated helpful exchanges".into(),
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        let rel = &state.bonds[&person];
        assert_eq!(rel.closeness, 0.4);
        assert_eq!(rel.reliability, 1.0);
        assert_eq!(rel.reciprocity, 0.3);
        assert_eq!(rel.conflict_level, 0.0);
    }

    #[test]
    fn proactive_outbound_relationship_delta_tracks_unanswered_outreach() {
        let person = PersonId("person-proactive".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: Some(RelationshipInteraction::ProactiveOutbound),
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        let rel = &state.bonds[&person];
        assert_eq!(rel.interaction_count, 1);
        assert_eq!(rel.outbound_count, 1);
        assert_eq!(rel.proactive_outbound_count, 1);
        assert!(rel.last_outbound > 0);
        assert_eq!(rel.last_proactive_outbound, rel.last_outbound);
    }

    #[test]
    fn merge_person_context_reconciles_relationship_state() {
        let from = PersonId("person-claimant".into());
        let into = PersonId("person-verified".into());
        let mut state = ActorState::new(CoreTraits::default());
        state.set_relationship_config(&into, Some(Authority::Trusted));
        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: into.clone(),
                    trust_delta: 0.2,
                    trust_ceiling: None,
                    familiarity_delta: 0.2,
                    valence_delta: 0.2,
                    proactive_consent: None,
                    response_cadence: Some("same business day".into()),
                    channel_preference: None,
                    interaction: Some(RelationshipInteraction::ProactiveOutbound),
                }],
                relationship_signal_updates: vec![RelationshipSignalUpdate {
                    person: into.clone(),
                    closeness_delta: 0.2,
                    reliability_delta: 0.4,
                    reciprocity_delta: 0.3,
                    conflict_delta: 0.0,
                    reason: "known reliable collaborator".into(),
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );
        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: from.clone(),
                    trust_delta: 0.3,
                    trust_ceiling: None,
                    familiarity_delta: 0.5,
                    valence_delta: -0.1,
                    proactive_consent: Some(ProactiveConsent::Allowed),
                    response_cadence: Some("within the week".into()),
                    channel_preference: Some("Discord for routine updates".into()),
                    interaction: Some(RelationshipInteraction::Inbound),
                }],
                relationship_signal_updates: vec![RelationshipSignalUpdate {
                    person: from.clone(),
                    closeness_delta: 0.6,
                    reliability_delta: 0.1,
                    reciprocity_delta: 0.7,
                    conflict_delta: 0.2,
                    reason: "claimant context before merge".into(),
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        state.merge_person_context(&from, &into);

        assert!(!state.bonds.contains_key(&from));
        assert_eq!(state.bonds[&into].authority, Authority::Trusted);
        assert!(state.bonds[&into].trust <= Authority::Trusted.trust_ceiling());
        assert_eq!(state.bonds[&into].familiarity, 0.5);
        assert_eq!(state.bonds[&into].interaction_count, 2);
        assert_eq!(state.bonds[&into].inbound_count, 1);
        assert_eq!(state.bonds[&into].outbound_count, 1);
        assert_eq!(state.bonds[&into].proactive_outbound_count, 1);
        assert!(state.bonds[&into].last_proactive_outbound > 0);
        assert_eq!(
            state.bonds[&into].proactive_consent,
            ProactiveConsent::Allowed
        );
        assert_eq!(state.bonds[&into].closeness, 0.6);
        assert_eq!(state.bonds[&into].reliability, 0.4);
        assert_eq!(state.bonds[&into].reciprocity, 0.7);
        assert_eq!(state.bonds[&into].conflict_level, 0.2);
        assert_eq!(
            state.bonds[&into].response_cadence.as_deref(),
            Some("same business day")
        );
        assert_eq!(
            state.bonds[&into].channel_preference.as_deref(),
            Some("Discord for routine updates")
        );
    }

    #[test]
    fn relationship_delta_updates_response_and_channel_preferences() {
        let person = PersonId("person-preference".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: Some("  reply within one business day  ".into()),
                    channel_preference: Some("  Discord for quick coordination  ".into()),
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        let rel = &state.bonds[&person];
        assert_eq!(
            rel.response_cadence.as_deref(),
            Some("reply within one business day")
        );
        assert_eq!(
            rel.channel_preference.as_deref(),
            Some("Discord for quick coordination")
        );

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.0,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: Some(" ".into()),
                    channel_preference: Some("\n\t".into()),
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        let rel = &state.bonds[&person];
        assert_eq!(
            rel.response_cadence.as_deref(),
            Some("reply within one business day")
        );
        assert_eq!(
            rel.channel_preference.as_deref(),
            Some("Discord for quick coordination")
        );
    }

    #[test]
    fn qualitative_relationship_delta_does_not_count_as_interaction() {
        let person = PersonId("person-sam".into());
        let mut state = ActorState::new(CoreTraits::default());

        state.apply_delta(
            &Delta {
                relationship_changes: vec![RelationshipChange {
                    person: person.clone(),
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.2,
                    valence_delta: 0.1,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );

        let rel = &state.bonds[&person];
        assert_eq!(rel.interaction_count, 0);
        assert_eq!(rel.last_interaction, 0);
        assert_eq!(rel.inbound_count, 0);
        assert_eq!(rel.outbound_count, 0);
        assert_eq!(rel.proactive_outbound_count, 0);
        assert_eq!(rel.familiarity, 0.2);
    }
}
