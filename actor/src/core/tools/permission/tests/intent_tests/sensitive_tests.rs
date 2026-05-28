use super::*;

#[tokio::test]
async fn sensitive_proactive_intent_creates_can_be_routed_for_chosen_person_approval() {
    let mut current = test_context(Authority::Default, ActionKind::Respond);
    current.messages[0].person = Some(PersonId("person-current".into()));

    check(
        "create_intent",
        &serde_json::json!({
            "task": "Ask Sam about the private medical update",
            "kind": "scheduled",
            "fire_at": 1200,
            "person": "person-current"
        }),
        &current,
    )
    .await
    .unwrap();

    let review = test_context(Authority::Default, ActionKind::Review);
    check(
        "create_intent",
        &serde_json::json!({
            "task": "Follow up about the confidential family issue",
            "kind": "scheduled",
            "fire_at": 1200,
            "requires_chosen_person_approval": true
        }),
        &review,
    )
    .await
    .unwrap();

    let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
    check(
        "create_intent",
        &serde_json::json!({
            "task": "Follow up about the private medical update",
            "kind": "scheduled",
            "fire_at": 1200,
            "sensitive": true
        }),
        &chosen_person,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn sensitive_intent_updates_require_chosen_person_authority() {
    let review = test_context(Authority::Default, ActionKind::Review);
    let denied = check(
        "update_intent",
        &serde_json::json!({
            "intent_id": "intent-1",
            "task": "Follow up about a bank payment",
            "sensitive": true
        }),
        &review,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("Sensitive proactive outreach"));

    let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
    check(
        "update_intent",
        &serde_json::json!({
            "intent_id": "intent-1",
            "task": "Follow up about a bank payment",
            "requires_chosen_person_approval": true
        }),
        &chosen_person,
    )
    .await
    .unwrap();
}
#[tokio::test]
async fn pending_chosen_person_approval_intents_can_only_be_activated_by_chosen_person() {
    let review = test_context(Authority::Default, ActionKind::Review);
    review
        .store
        .create_intent(&IntentRecord {
            id: "intent-pending-chosen-person-approval".into(),
            kind: "scheduled".into(),
            status: "pending_approval".into(),
            task: "Ask Sam about the private medical update".into(),
            person: None,
            profile: None,
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
            "intent_id": "intent-pending-chosen-person-approval",
            "status": "active"
        }),
        &review,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("chosen-person authority"));

    let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
    chosen_person
        .store
        .create_intent(&IntentRecord {
            id: "intent-pending-chosen-person-approval".into(),
            kind: "scheduled".into(),
            status: "pending_approval".into(),
            task: "Ask Sam about the private medical update".into(),
            person: None,
            profile: None,
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
    check(
        "update_intent",
        &serde_json::json!({
            "intent_id": "intent-pending-chosen-person-approval",
            "status": "active"
        }),
        &chosen_person,
    )
    .await
    .unwrap();
}
