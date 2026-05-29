use super::*;

#[tokio::test]
async fn default_user_cannot_send_explicit_outbound_to_other_target() {
    let ctx = test_context(RelationshipStanding::Default, ActionKind::Respond);

    let denied = check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "discord",
            "external_id": "channel-2"
        }),
        &ctx,
    )
    .await
    .unwrap_err();

    assert!(denied.contains("Explicit outbound messaging requires"));
}
#[tokio::test]
async fn explicit_current_reply_target_is_allowed() {
    let ctx = test_context(RelationshipStanding::Default, ActionKind::Respond);

    check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "relay",
            "external_id": "local"
        }),
        &ctx,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn background_actions_cannot_send_visible_messages() {
    for kind in [
        ActionKind::Review,
        ActionKind::Research,
        ActionKind::Consolidate,
        ActionKind::Ruminate,
    ] {
        let ctx = test_context(RelationshipStanding::ChosenHuman, kind);
        let denied = check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "relay",
                "external_id": "local"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("internal/background"));
    }
}
#[tokio::test]
async fn trusted_context_can_send_explicit_outbound() {
    let trusted = test_context(RelationshipStanding::Trusted, ActionKind::Respond);
    check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "discord",
            "external_id": "channel-2"
        }),
        &trusted,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn outreach_context_can_only_send_explicit_scheduled_target() {
    let outreach = test_context(RelationshipStanding::Default, ActionKind::Outreach);
    check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "relay",
            "external_id": "local"
        }),
        &outreach,
    )
    .await
    .unwrap();

    let denied = check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "discord",
            "external_id": "channel-2"
        }),
        &outreach,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("scheduled outreach target"));
}
#[tokio::test]
async fn outreach_without_current_messages_uses_stored_conversation_target() {
    let mut outreach = test_context(RelationshipStanding::Default, ActionKind::Outreach);
    let conversation = ConversationId("relay:outreach".into());
    outreach.messages.clear();
    outreach.conversation = Some(conversation.clone());
    outreach
        .store
        .append_message(
            &conversation,
            Some("relay"),
            None,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "previous outreach context".into(),
                identity: None,
                profile: None,
                person: Some(PersonId("person-target".into())),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-outreach-context".into()),
                sender_external_id: Some("target-1".into()),
                reply_external_id: Some("target-1".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "relay",
            "external_id": "target-1"
        }),
        &outreach,
    )
    .await
    .unwrap();

    let denied = check(
        "send_message",
        &serde_json::json!({
            "content": "hi",
            "gateway_id": "relay",
            "external_id": "target-2"
        }),
        &outreach,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("scheduled outreach target"));
}
