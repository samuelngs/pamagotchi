use super::*;

#[tokio::test]
async fn default_user_cannot_write_structured_identity_memory() {
    let ctx = test_context(Authority::Default, ActionKind::Respond);

    let denied = check(
        "form_memory",
        &serde_json::json!({
            "content": "I am a different core identity now.",
            "memory_type": "identity_claim"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("core about yourself"));

    let denied = check(
        "form_memory",
        &serde_json::json!({
            "content": "My private identity marker is different.",
            "sensitivity_category": "identity"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("core about yourself"));

    let denied = check(
        "form_memory",
        &serde_json::json!({
            "content": "My name is Pamagotchi.",
            "kind": "semantic",
            "subject_actor": true
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("core about yourself"));
}
#[tokio::test]
async fn chosen_human_can_write_structured_identity_memory() {
    let ctx = test_context(Authority::ChosenHuman, ActionKind::Respond);

    check(
        "form_memory",
        &serde_json::json!({
            "content": "My name is Pamagotchi.",
            "memory_type": "identity_claim",
            "sensitivity_category": "identity"
        }),
        &ctx,
    )
    .await
    .unwrap();
}
