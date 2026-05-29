use super::*;

#[tokio::test]
async fn due_intents_coalesce_by_target_per_scan() {
    let store = test_store();
    for (id, person, conversation, dedupe_key, priority) in [
        ("person-high", Some("sam"), Some("relay:sam"), None, 90u8),
        (
            "person-low",
            Some("sam"),
            Some("relay:sam-other"),
            None,
            70u8,
        ),
        ("conversation-high", None, Some("relay:local"), None, 80u8),
        ("conversation-low", None, Some("relay:local"), None, 60u8),
        ("dedupe-high", None, None, Some("followup:deploy"), 50u8),
        ("dedupe-low", None, None, Some("followup:deploy"), 40u8),
        ("unique-a", None, None, None, 30u8),
        ("unique-b", None, None, None, 20u8),
    ] {
        store
            .create_intent(&IntentRecord {
                id: id.into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: format!("{id} task"),
                person: person.map(|id| PersonId(id.into())),
                profile: None,
                conversation: conversation.map(|id| ConversationId(id.into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: None,
                priority,
                dedupe_key: dedupe_key.map(str::to_string),
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                chosen_human_approved: false,
            })
            .await
            .unwrap();
    }

    let due = store.due_intents(1000, 10).await.unwrap();
    let ids = due
        .iter()
        .map(|intent| intent.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            "person-high",
            "conversation-high",
            "dedupe-high",
            "unique-a",
            "unique-b"
        ]
    );
}
#[tokio::test]
async fn active_intents_for_context_returns_matching_open_loops() {
    let store = test_store();
    for (id, status, person, profile, conversation, fire_at, condition, priority) in [
        (
            "due-person",
            "active",
            Some("sam"),
            None,
            Some("relay:sam"),
            Some(1000),
            None,
            80u8,
        ),
        (
            "future-profile",
            "active",
            None,
            Some("profile-sam"),
            None,
            Some(2000),
            None,
            70u8,
        ),
        (
            "conversation-trigger",
            "active",
            None,
            None,
            Some("relay:local"),
            None,
            Some("next time Sam asks about deployment"),
            90u8,
        ),
        (
            "global-open-loop",
            "active",
            None,
            None,
            None,
            None,
            Some("when any conversation mentions deployment"),
            60u8,
        ),
        (
            "other-person",
            "active",
            Some("alice"),
            None,
            None,
            Some(900),
            None,
            100u8,
        ),
        (
            "cancelled-current",
            "cancelled",
            Some("sam"),
            None,
            None,
            Some(900),
            None,
            100u8,
        ),
    ] {
        store
            .create_intent(&IntentRecord {
                id: id.into(),
                kind: if condition.is_some() {
                    "triggered".into()
                } else {
                    "scheduled".into()
                },
                status: status.into(),
                task: format!("{id} task"),
                person: person.map(|id| PersonId(id.into())),
                profile: profile.map(|id| ProfileId(id.into())),
                conversation: conversation.map(|id| ConversationId(id.into())),
                fire_at,
                condition: condition.map(str::to_string),
                recurrence: None,
                priority,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 800,
                updated_at: 800,
                last_fired_at: None,
                chosen_human_approved: false,
            })
            .await
            .unwrap();
    }

    let active = store
        .active_intents_for_context(
            Some(&PersonId("sam".into())),
            Some(&ProfileId("profile-sam".into())),
            Some(&ConversationId("relay:local".into())),
            1000,
            10,
        )
        .await
        .unwrap();
    let ids = active
        .iter()
        .map(|intent| intent.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            "due-person",
            "conversation-trigger",
            "future-profile",
            "global-open-loop"
        ]
    );
}
