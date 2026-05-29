use super::*;

#[tokio::test]
async fn apply_review_can_create_current_group_directive() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-group".into());
    let person = PersonId("person-group".into());
    let conversation = ConversationId("relay:group".into());
    let group = GroupId("group-review".into());
    let (mut ctx, mut session_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    ctx.messages[0].group = Some(group.clone());

    let result = apply(
        &json!({
            "directives": [{
                "scope": "group",
                "group_id": group.0.clone(),
                "directive": "Use the group norm: keep release updates brief and action-oriented.",
                "priority": 12
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["directives"], 1);
    assert!(parsed["skipped"].as_array().unwrap().is_empty());
    let directives = store
        .get_directives_for_context(&person, &Authority::Default, Some(&group))
        .await
        .unwrap();
    assert_eq!(directives.len(), 1);
    assert_eq!(
        directives[0].scope.scope_value().as_deref(),
        Some("group-review")
    );
    assert_eq!(directives[0].set_by, person);
    assert_eq!(directives[0].priority, 12);
    assert!(directives[0].directive.contains("release updates brief"));
}
#[tokio::test]
async fn non_chosen_human_review_cannot_create_directives_outside_current_scope() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let profile = ProfileId("profile-group".into());
    let person = PersonId("person-group".into());
    let conversation = ConversationId("relay:group".into());
    let group = GroupId("group-current".into());
    let (mut ctx, mut session_state) =
        test_context(store.clone(), &profile, &person, &conversation);
    ctx.messages[0].group = Some(group);

    let result = apply(
        &json!({
            "directives": [{
                "scope": "group",
                "group_id": "group-other",
                "directive": "Use another group's norm."
            }, {
                "scope": "person",
                "person_id": "person-other",
                "directive": "Use another person's norm."
            }, {
                "scope": "global",
                "directive": "Use a global norm."
            }]
        }),
        &ctx,
        &mut session_state,
    )
    .await;
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["directives"], 0);
    assert_eq!(parsed["skipped"].as_array().unwrap().len(), 3);
    assert!(store.list_directives().await.unwrap().is_empty());
}
