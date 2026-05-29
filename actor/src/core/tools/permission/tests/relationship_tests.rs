use super::*;

#[tokio::test]
async fn relationship_trust_ceiling_requires_chosen_human_social_path_for_default_people() {
    let ctx = test_context(Authority::Default, ActionKind::Review);
    let chosen_human = PersonId("person-chosen_human".into());
    let stranger = PersonId("person-stranger".into());
    {
        let mut actor = ctx.state.shared.actor.write().unwrap();
        actor.set_relationship_config(&chosen_human, Some(Authority::ChosenHuman));
    }

    let ceiling = relationship_trust_ceiling(&ctx, &stranger).await;

    assert_eq!(ceiling, crate::state::Relationship::default().trust);
}
#[tokio::test]
async fn relationship_trust_ceiling_allows_chosen_human_connected_social_path() {
    let ctx = test_context(Authority::Default, ActionKind::Review);
    let chosen_human = PersonId("person-chosen_human".into());
    let middle = PersonId("person-middle".into());
    let connected = PersonId("person-connected".into());
    {
        let mut actor = ctx.state.shared.actor.write().unwrap();
        actor.set_relationship_config(&chosen_human, Some(Authority::ChosenHuman));
    }
    ctx.store
        .upsert_relation(&SocialRelation {
            person_a: chosen_human.clone(),
            person_b: middle.clone(),
            relation: Relation::Friend,
            direction: Relation::Friend.default_direction(),
            confidence: 0.9,
            status: RelationStatus::Confirmed,
            evidence: Some(serde_json::json!({"source": "test"})),
            source_kind: RelationSource::ChosenHumanConfirmed,
            asserted_by: Some(chosen_human.clone()),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    ctx.store
        .upsert_relation(&SocialRelation {
            person_a: middle.clone(),
            person_b: connected.clone(),
            relation: Relation::Coworker,
            direction: Relation::Coworker.default_direction(),
            confidence: 0.8,
            status: RelationStatus::Stated,
            evidence: Some(serde_json::json!({"source": "test"})),
            source_kind: RelationSource::Stated,
            asserted_by: Some(middle.clone()),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();

    let ceiling = relationship_trust_ceiling(&ctx, &connected).await;

    assert_eq!(ceiling, Authority::Default.trust_ceiling());
}
#[tokio::test]
async fn default_user_cannot_update_other_profile_or_person() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
    ctx.messages[0].person = Some(PersonId("person-current".into()));

    let profile_denied = check(
        "update_profile",
        &serde_json::json!({
            "ref": "profile-other",
            "summary": "Cross-profile summary"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(profile_denied.contains("another profile"));

    let person_denied = check(
        "update_person",
        &serde_json::json!({
            "ref": "person-other",
            "summary": "Cross-person summary"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(person_denied.contains("another person"));
}
#[tokio::test]
async fn current_profile_and_person_updates_are_allowed() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
    ctx.messages[0].person = Some(PersonId("person-current".into()));

    check(
        "update_profile",
        &serde_json::json!({
            "ref": "profile-current",
            "summary": "Current profile summary"
        }),
        &ctx,
    )
    .await
    .unwrap();

    check(
        "update_person",
        &serde_json::json!({
            "ref": "person-current",
            "summary": "Current person summary"
        }),
        &ctx,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn live_reflection_relationship_changes_are_current_person_only() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].person = Some(PersonId("person-current".into()));

    check(
        "reflect",
        &serde_json::json!({
            "relationship_changes": [{
                "person": "person-current",
                "trust_delta": 0.01,
                "familiarity_delta": 0.02,
                "valence_delta": 0.01
            }]
        }),
        &ctx,
    )
    .await
    .unwrap();

    let denied = check(
        "reflect",
        &serde_json::json!({
            "relationship_changes": [{
                "person": "person-other",
                "trust_delta": 0.01,
                "familiarity_delta": 0.02,
                "valence_delta": 0.01
            }]
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("another person"));

    let chosen_human = test_context(Authority::ChosenHuman, ActionKind::Respond);
    check(
        "reflect",
        &serde_json::json!({
            "relationship_changes": [{
                "person": "person-other",
                "trust_delta": 0.01
            }]
        }),
        &chosen_human,
    )
    .await
    .unwrap();

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "reflect",
        &serde_json::json!({
            "relationship_changes": [{
                "person": "person-other",
                "trust_delta": 0.01
            }]
        }),
        &review,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn chosen_human_or_review_can_update_other_profile_and_person() {
    let chosen_human = test_context(Authority::ChosenHuman, ActionKind::Respond);
    check(
        "update_profile",
        &serde_json::json!({
            "ref": "profile-other",
            "summary": "Chosen-human-visible profile summary"
        }),
        &chosen_human,
    )
    .await
    .unwrap();

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "update_person",
        &serde_json::json!({
            "ref": "person-other",
            "summary": "Review-supported person summary"
        }),
        &review,
    )
    .await
    .unwrap();
}
