use super::*;

#[tokio::test]
async fn apply_review_can_create_triggered_open_loop_without_fire_at() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

    let review_args = json!({
        "open_loops": [{
            "kind": "triggered",
            "task": "Ask how the deployment went",
            "condition": "next time Sam messages",
            "conversation_id": conversation.0,
            "dedupe_key": "review:test:triggered-followup"
        }]
    });

    let result = apply(&review_args, &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["open_loops"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    assert!(
        store
            .due_intents(util::now() + 3600, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let active = store
        .active_intents_for_context(Some(&person), Some(&profile), Some(&conversation), 0, 10)
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].kind, "triggered");
    assert_eq!(active[0].task, "Ask how the deployment went");
    assert_eq!(
        active[0].condition.as_deref(),
        Some("next time Sam messages")
    );
    assert!(active[0].fire_at.is_none());
}
#[tokio::test]
async fn apply_review_accepts_follow_up_open_loop_alias() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let conversation = ConversationId("relay:local".into());
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
    let now = util::now();

    let review_args = json!({
        "open_loops": [
            {
                "kind": "follow_up",
                "task": "Check whether the deployment finished",
                "fire_at": now + 3600,
                "conversation_id": conversation.0,
                "dedupe_key": "review:test:follow-up-scheduled"
            },
            {
                "kind": "follow_up",
                "task": "Ask about deployment blockers",
                "condition": "next time Sam mentions deployment",
                "conversation_id": conversation.0,
                "dedupe_key": "review:test:follow-up-triggered"
            }
        ]
    });

    let result = apply(&review_args, &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["open_loops"], 2);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let due = store.due_intents(now + 3600, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].kind, "scheduled");
    assert_eq!(due[0].task, "Check whether the deployment finished");
    assert_eq!(due[0].fire_at, Some(now + 3600));
    assert!(due[0].condition.is_none());

    let active = store
        .active_intents_for_context(Some(&person), Some(&profile), Some(&conversation), 0, 10)
        .await
        .unwrap();
    let triggered = active
        .iter()
        .find(|intent| intent.task == "Ask about deployment blockers")
        .unwrap();
    assert_eq!(triggered.kind, "triggered");
    assert_eq!(
        triggered.condition.as_deref(),
        Some("next time Sam mentions deployment")
    );
    assert!(triggered.fire_at.is_none());
    assert!(active.iter().all(|intent| intent.kind != "follow_up"));
}
#[tokio::test]
async fn apply_review_routes_sensitive_open_loop_to_chosen_human_approval_intent() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-sam".into());
    let person = PersonId("person-sam".into());
    let chosen_human = PersonId("person-chosen_human".into());
    let conversation = ConversationId("relay:local".into());
    let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
    ctx.state
        .shared
        .actor
        .write()
        .unwrap()
        .set_relationship_config(&chosen_human, Some(Authority::ChosenHuman));

    let review_args = json!({
        "open_loops": [{
            "task": "Ask about the private medical update",
            "fire_at": util::now() + 3600,
            "conversation_id": conversation.0,
            "sensitive": true,
            "source_memory_id": "memory-sensitive-medical-update",
            "dedupe_key": "review:test:sensitive-followup"
        }]
    });

    let result = apply(&review_args, &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["open_loops"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let due = store.due_intents(util::now() + 1, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].person.as_ref(), Some(&chosen_human));
    assert!(due[0].chosen_human_approved);
    assert_eq!(due[0].priority, 100);
    assert!(due[0].task.contains("Review sensitive proactive outreach"));
    assert!(due[0].task.contains("Ask about the private medical update"));
    assert!(due[0].task.contains("Pending intent:"));
    assert!(due[0].task.contains("update intent"));
    assert_eq!(
        due[0].source_memory.as_ref().map(|id| id.0.as_str()),
        Some("memory-sensitive-medical-update")
    );

    let pending_id = due[0]
        .task
        .split("Pending intent: ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .expect("pending intent id in chosen-human approval task")
        .to_string();
    assert!(due[0].task.contains(&pending_id));
    let pending = store.get_intent(&pending_id).await.unwrap().unwrap();
    assert_eq!(pending.status, "pending_approval");
    assert_eq!(pending.task, "Ask about the private medical update");
    assert_eq!(pending.person.as_ref(), Some(&person));
    assert_eq!(pending.profile.as_ref(), Some(&profile));
    assert_eq!(pending.conversation.as_ref(), Some(&conversation));
    assert!(!pending.chosen_human_approved);
    assert_eq!(
        pending.source_memory.as_ref().map(|id| id.0.as_str()),
        Some("memory-sensitive-medical-update")
    );

    let target_intents = store
        .active_intents_for_context(Some(&person), Some(&profile), Some(&conversation), 0, 10)
        .await
        .unwrap();
    assert!(
        target_intents
            .iter()
            .all(|intent| !intent.task.contains("private medical update"))
    );

    let (mut chosen_human_ctx, mut chosen_human_state) =
        test_context(store.clone(), &profile, &chosen_human, &conversation);
    chosen_human_ctx.authority = Authority::ChosenHuman;
    let update_result = match crate::core::tools::execute(
        "update_intent",
        &json!({
            "intent_id": pending_id,
            "status": "active"
        }),
        &chosen_human_ctx,
        &mut chosen_human_state,
    )
    .await
    {
        crate::core::tools::ToolOutcome::Result(result) => result,
        crate::core::tools::ToolOutcome::Decision(_) => {
            panic!("update_intent should produce a tool result")
        }
    };
    let parsed_update: Value = serde_json::from_str(&update_result).unwrap();
    assert_eq!(parsed_update["status"], "updated");
    let approved = store.get_intent(&pending.id).await.unwrap().unwrap();
    assert_eq!(approved.status, "active");
    assert!(approved.chosen_human_approved);
}
#[tokio::test]
async fn chosen_human_review_can_create_third_party_open_loop() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-chosen_human".into());
    let person = PersonId("person-chosen_human".into());
    let conversation = ConversationId("relay:chosen_human".into());
    let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
    ctx.authority = Authority::ChosenHuman;

    let now = util::now();
    let review_args = json!({
        "open_loops": [{
            "task": "Remind Alice to bring the deployment checklist",
            "fire_at": now + 3600,
            "person_id": "person-alice",
            "profile_id": "profile-alice",
            "conversation_id": "relay:alice",
            "dedupe_key": "chosen_human:remind-alice-checklist"
        }]
    });

    let result = apply(&review_args, &ctx, &mut state).await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["open_loops"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());

    let due = store.due_intents(now + 3600, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].person.as_ref(),
        Some(&PersonId("person-alice".into()))
    );
    assert_eq!(
        due[0].profile.as_ref(),
        Some(&ProfileId("profile-alice".into()))
    );
    assert_eq!(
        due[0].conversation.as_ref(),
        Some(&ConversationId("relay:alice".into()))
    );
    assert!(due[0].chosen_human_approved);
}
