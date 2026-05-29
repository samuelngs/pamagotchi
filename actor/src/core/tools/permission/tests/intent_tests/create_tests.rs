use super::*;

#[tokio::test]
async fn default_user_can_create_current_target_intent() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
    ctx.messages[0].person = Some(PersonId("person-current".into()));

    check(
        "create_intent",
        &serde_json::json!({
            "task": "Follow up here later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-current",
            "profile": "profile-current",
            "conversation": "relay:local"
        }),
        &ctx,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn default_user_cannot_create_cross_target_intent() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].person = Some(PersonId("person-current".into()));

    let denied = check(
        "create_intent",
        &serde_json::json!({
            "task": "Message Alice later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-alice"
        }),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("another person"));
}
#[tokio::test]
async fn chosen_human_can_create_cross_target_intent_and_review_requires_verified_target() {
    let chosen_human = test_context(Authority::ChosenHuman, ActionKind::Respond);
    check(
        "create_intent",
        &serde_json::json!({
            "task": "Message Alice later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-alice"
        }),
        &chosen_human,
    )
    .await
    .unwrap();

    let review = test_context(Authority::Default, ActionKind::Review);
    let denied = check(
        "create_intent",
        &serde_json::json!({
            "task": "Message Alice later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-alice"
        }),
        &review,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Third-party proactive outreach"));

    add_verified_target(
        &review,
        &ProfileId("profile-alice".into()),
        &PersonId("person-alice".into()),
    )
    .await;
    check(
        "create_intent",
        &serde_json::json!({
            "task": "Message Alice later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-alice",
            "profile": "profile-alice"
        }),
        &review,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn ruminate_can_create_verified_target_intent_but_not_unverified_target() {
    let mut ruminate = test_context(Authority::Default, ActionKind::Ruminate);
    ruminate.messages.clear();
    ruminate.conversation = None;

    let denied = check(
        "create_intent",
        &serde_json::json!({
            "task": "Check in with Alice later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-alice"
        }),
        &ruminate,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("verified target profile"));

    add_verified_target(
        &ruminate,
        &ProfileId("profile-alice".into()),
        &PersonId("person-alice".into()),
    )
    .await;
    check(
        "create_intent",
        &serde_json::json!({
            "task": "Check in with Alice later",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-alice",
            "profile": "profile-alice"
        }),
        &ruminate,
    )
    .await
    .unwrap();
}
