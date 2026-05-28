use super::*;

#[tokio::test]
async fn apply_review_is_review_only() {
    let respond = test_context(Authority::ChosenPerson, ActionKind::Respond);
    let denied = check("apply_review", &serde_json::json!({}), &respond)
        .await
        .unwrap_err();
    assert!(denied.contains("review actions"));

    let review = test_context(Authority::Default, ActionKind::Review);
    check("apply_review", &serde_json::json!({}), &review)
        .await
        .unwrap();
}
#[tokio::test]
async fn conversation_summary_updates_are_current_or_privileged() {
    let current = test_context(Authority::Default, ActionKind::Respond);
    check(
        "update_conversation_summary",
        &serde_json::json!({
            "conversation": "relay:local",
            "summary": "Current conversation summary."
        }),
        &current,
    )
    .await
    .unwrap();

    let denied = check(
        "update_conversation_summary",
        &serde_json::json!({
            "conversation": "relay:other",
            "summary": "Other conversation summary."
        }),
        &current,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("another conversation summary"));

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "update_conversation_summary",
        &serde_json::json!({
            "conversation": "relay:other",
            "summary": "Review can summarize backlog."
        }),
        &review,
    )
    .await
    .unwrap();

    let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
    check(
        "update_conversation_summary",
        &serde_json::json!({
            "conversation": "relay:other",
            "summary": "Chosen-person-directed summary maintenance."
        }),
        &chosen_person,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn message_reads_are_current_or_privileged() {
    let current = test_context(Authority::Default, ActionKind::Respond);
    check(
        "read_messages",
        &serde_json::json!({
            "conversation": "relay:local",
            "limit": 5
        }),
        &current,
    )
    .await
    .unwrap();

    check("read_messages", &serde_json::json!({"limit": 5}), &current)
        .await
        .unwrap();

    let mut no_current = test_context(Authority::Default, ActionKind::Respond);
    no_current.messages.clear();
    no_current.conversation = None;
    let denied = check(
        "read_messages",
        &serde_json::json!({"limit": 5}),
        &no_current,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("without a current conversation"));

    let mut ruminate = test_context(Authority::Default, ActionKind::Ruminate);
    ruminate.messages.clear();
    ruminate.conversation = None;
    check("read_messages", &serde_json::json!({"limit": 5}), &ruminate)
        .await
        .unwrap();

    let denied = check(
        "read_messages",
        &serde_json::json!({
            "conversation": "relay:other",
            "limit": 5
        }),
        &current,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Reading another conversation"));

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "read_messages",
        &serde_json::json!({
            "conversation": "relay:other",
            "limit": 5
        }),
        &review,
    )
    .await
    .unwrap();

    let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
    check(
        "read_messages",
        &serde_json::json!({
            "conversation": "relay:other",
            "limit": 5
        }),
        &chosen_person,
    )
    .await
    .unwrap();
}
