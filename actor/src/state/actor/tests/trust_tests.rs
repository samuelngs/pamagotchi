use super::*;

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

    assert_eq!(
        state.bonds[&person].relationship_standing,
        RelationshipStanding::Default
    );
    assert_eq!(
        state.bonds[&person].trust,
        RelationshipStanding::Default.trust_ceiling()
    );
}
#[test]
fn lowering_relationship_standing_lowers_existing_trust_ceiling() {
    let person = PersonId("person-restricted".into());
    let mut state = ActorState::new(CoreTraits::default());
    state.set_relationship_config(&person, Some(RelationshipStanding::ChosenHuman));
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

    state.set_relationship_config(&person, Some(RelationshipStanding::Restricted));

    assert_eq!(
        state.bonds[&person].trust,
        RelationshipStanding::Restricted.trust_ceiling()
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
