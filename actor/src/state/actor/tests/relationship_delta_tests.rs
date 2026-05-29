use super::*;

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
    state.set_relationship_config(&into, Some(RelationshipStanding::Trusted));
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
    assert_eq!(
        state.bonds[&into].relationship_standing,
        RelationshipStanding::Trusted
    );
    assert!(state.bonds[&into].trust <= RelationshipStanding::Trusted.trust_ceiling());
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
