use super::*;

#[tokio::test]
async fn default_user_cannot_update_social_graph() {
    let ctx = test_context(RelationshipStanding::Default, ActionKind::Respond);

    let denied = check(
        "upsert_social_relation",
        &serde_json::json!({
            "person_a": "person-a",
            "person_b": "person-b",
            "relation": "friend"
        }),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("Social graph updates require"));
}
#[tokio::test]
async fn review_can_update_social_graph_but_not_chosen_human_confirm() {
    let mut ctx = test_context(RelationshipStanding::Default, ActionKind::Review);
    let current_profile = ProfileId("profile-a".into());
    let current_person = PersonId("person-a".into());
    add_verified_target(&ctx, &current_profile, &current_person).await;
    ctx.messages[0].profile = Some(current_profile);
    ctx.messages[0].person = Some(current_person);

    check(
        "upsert_social_relation",
        &serde_json::json!({
            "person_a": "person-a",
            "person_b": "person-b",
            "relation": "friend",
            "source_kind": "stated"
        }),
        &ctx,
    )
    .await
    .unwrap();

    let denied_third_party = check(
        "upsert_social_relation",
        &serde_json::json!({
            "person_a": "person-b",
            "person_b": "person-c",
            "relation": "friend",
            "source_kind": "stated"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied_third_party.contains("current strongly verified person"));

    let denied = check(
        "upsert_social_relation",
        &serde_json::json!({
            "person_a": "person-a",
            "person_b": "person-b",
            "relation": "friend",
            "source_kind": "chosen_human_confirmed"
        }),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("Chosen-human-confirmed"));
}
