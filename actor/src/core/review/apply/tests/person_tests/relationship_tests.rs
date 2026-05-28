use super::*;

#[tokio::test]
async fn apply_review_allows_current_person_restrictive_relationship_delta() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-weak".into());
    let person = PersonId("person-weak".into());
    let conversation = ConversationId("relay:weak".into());
    let (ctx, mut session_state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "relationship_delta": [{
                "person_id": person.0,
                "trust_delta": -0.5,
                "familiarity_delta": 0.0,
                "valence_delta": -0.5,
                "proactive_consent": "denied",
                "reason": "current person asked not to receive proactive outreach"
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["relationship_deltas"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());
    assert_eq!(session_state.delta.relationship_changes.len(), 1);
    assert_eq!(
        session_state.delta.relationship_changes[0].proactive_consent,
        Some(ProactiveConsent::Denied)
    );
    assert_eq!(
        session_state.delta.relationship_changes[0].trust_delta,
        -0.05
    );
}
#[tokio::test]
async fn apply_review_requires_verified_anchor_for_relationship_preferences() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-weak".into());
    let person = PersonId("person-weak".into());
    let conversation = ConversationId("relay:weak".into());
    let (ctx, mut session_state) = test_context(store.clone(), &profile, &person, &conversation);

    let result = apply(
        &json!({
            "relationship_delta": [{
                "person_id": person.0,
                "response_cadence": "reply within one business day",
                "channel_preference": "Discord for quick coordination",
                "reason": "current message implies durable delivery preferences"
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["relationship_deltas"], 0);
    assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|message| message.contains("relationship_delta"))
    }));
    assert!(session_state.delta.relationship_changes.is_empty());
}
#[tokio::test]
async fn apply_review_sets_social_trust_ceiling_for_positive_relationship_delta() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let chosen_person = PersonId("person-chosen_person".into());
    let conversation = ConversationId("relay:local".into());
    let now = util::now();
    store
        .add_profile(&Profile {
            id: profile.clone(),
            display_name: Some("Sam".into()),
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .add_person(&Person {
            id: person.clone(),
            name: Some("Sam".into()),
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
        })
        .await
        .unwrap();
    store
        .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
        .await
        .unwrap();
    let (ctx, mut session_state) = test_context(store.clone(), &profile, &person, &conversation);
    {
        let mut actor = ctx.state.shared.actor.write().unwrap();
        actor.set_relationship_config(&chosen_person, Some(crate::state::Authority::ChosenPerson));
    }

    let result = apply(
        &json!({
            "relationship_delta": [{
                "person_id": person.0,
                "trust_delta": 0.5,
                "familiarity_delta": 0.2,
                "valence_delta": 0.1,
                "reason": "friendly but not chosen-person-connected"
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["relationship_deltas"], 1);
    assert_eq!(session_state.delta.relationship_changes.len(), 1);
    assert_eq!(
        session_state.delta.relationship_changes[0].trust_ceiling,
        Some(crate::state::Relationship::default().trust)
    );
}
