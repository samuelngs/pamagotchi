use super::*;

#[tokio::test]
async fn default_user_cannot_opt_into_sensitive_memory_recall() {
    let ctx = test_context(Authority::Default, ActionKind::Respond);

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "deployment credentials",
            "include_sensitive": true
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Sensitive memory recall requires"));

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "private detail",
            "max_sensitivity": 0.95
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Sensitive memory recall requires"));
}
#[tokio::test]
async fn chosen_human_or_review_can_opt_into_sensitive_memory_recall() {
    let chosen_human = test_context(Authority::ChosenHuman, ActionKind::Respond);
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "deployment credentials",
            "include_sensitive": true
        }),
        &chosen_human,
    )
    .await
    .unwrap();

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "private detail",
            "max_sensitivity": 0.95
        }),
        &review,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn default_user_memory_recall_is_current_target_only() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].identity = Some(IdentityId("identity-current".into()));
    ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
    ctx.messages[0].person = Some(PersonId("person-current".into()));

    check(
        "recall_memories",
        &serde_json::json!({"query": "current context"}),
        &ctx,
    )
    .await
    .unwrap();
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "current person preference",
            "person": "person-current"
        }),
        &ctx,
    )
    .await
    .unwrap();
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "current profile preference",
            "profile": "profile-current"
        }),
        &ctx,
    )
    .await
    .unwrap();
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "current identity preference",
            "identity": "identity-current"
        }),
        &ctx,
    )
    .await
    .unwrap();

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "other person's preferences",
            "person": "person-other"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("outside the current identity"));

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "other profile preferences",
            "profile": "profile-other"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("outside the current identity"));

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "other identity preferences",
            "identity": "identity-other"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("outside the current identity"));

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "anything",
            "scope": "global"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("outside the current identity"));
}
#[tokio::test]
async fn chosen_human_or_review_can_recall_outside_current_target() {
    let chosen_human = test_context(Authority::ChosenHuman, ActionKind::Respond);
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "other person preference",
            "person": "person-other"
        }),
        &chosen_human,
    )
    .await
    .unwrap();

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "recall_memories",
        &serde_json::json!({
            "query": "cross-profile duplicate",
            "scope": "global"
        }),
        &review,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn ruminate_can_recall_without_current_target_but_not_sensitive() {
    let ctx = test_context(Authority::Default, ActionKind::Ruminate);

    check(
        "recall_memories",
        &serde_json::json!({"query": "idle thought"}),
        &ctx,
    )
    .await
    .unwrap();

    let denied = check(
        "recall_memories",
        &serde_json::json!({
            "query": "private idle thought",
            "include_sensitive": true
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Sensitive memory recall requires"));
}
