use super::*;

#[tokio::test]
async fn ruminate_cannot_update_or_cancel_cross_target_intent() {
    let ruminate = test_context(Authority::Default, ActionKind::Ruminate);
    add_verified_target(
        &ruminate,
        &ProfileId("profile-alice".into()),
        &PersonId("person-alice".into()),
    )
    .await;
    ruminate
        .store
        .create_intent(&IntentRecord {
            id: "intent-alice".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Follow up with Alice".into(),
            person: Some(PersonId("person-alice".into())),
            profile: Some(ProfileId("profile-alice".into())),
            conversation: None,
            fire_at: Some(1200),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1000,
            updated_at: 1000,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let denied = check(
        "update_intent",
        &serde_json::json!({
            "intent_id": "intent-alice",
            "task": "Change Alice follow-up"
        }),
        &ruminate,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Updating intents"));

    let denied = check(
        "delete_intent",
        &serde_json::json!({
            "intent_id": "intent-alice"
        }),
        &ruminate,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Cancelling intents"));
}
#[tokio::test]
async fn default_user_cannot_update_or_cancel_cross_target_intent() {
    let mut ctx = test_context(Authority::Default, ActionKind::Respond);
    ctx.messages[0].person = Some(PersonId("person-current".into()));
    ctx.store
        .create_intent(&IntentRecord {
            id: "intent-other".into(),
            kind: "scheduled".into(),
            status: "active".into(),
            task: "Follow up with someone else".into(),
            person: Some(PersonId("person-other".into())),
            profile: None,
            conversation: Some(ConversationId("relay:other".into())),
            fire_at: Some(1200),
            condition: None,
            recurrence: None,
            priority: 50,
            dedupe_key: None,
            source_action: None,
            source_memory: None,
            created_at: 1000,
            updated_at: 1000,
            last_fired_at: None,
            chosen_person_approved: false,
        })
        .await
        .unwrap();

    let denied = check(
        "update_intent",
        &serde_json::json!({
            "intent_id": "intent-other",
            "task": "Change the other follow-up"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Updating intents"));

    let denied = check(
        "delete_intent",
        &serde_json::json!({
            "intent_id": "intent-other"
        }),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Cancelling intents"));
}
