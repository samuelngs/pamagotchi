use super::*;

#[test]
fn successful_visible_action_builds_one_post_turn_review() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    let action = Action::respond(
        vec![msg.clone()],
        msg.conversation.clone(),
        Authority::Default,
        None,
    );
    let injected = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-2",
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: true,
            attempted_send: true,
            review_messages: vec![injected.clone()],
            attempts: 1,
            ..Outcome::default()
        },
    );

    let review = mind
        .build_post_turn_review(&id)
        .expect("responded action should produce review");
    assert!(matches!(review.kind, ActionKind::Review));
    assert!(!review.kind.expects_response());
    assert_eq!(
        review.conversation,
        Some(ConversationId("relay:local".into()))
    );
    assert_eq!(review.source_messages.len(), 2);
    assert_eq!(review.source_messages[0].message_id, msg.message_id);
    assert_eq!(review.source_messages[1].message_id, injected.message_id);

    mind.reviewed_actions.insert(id.clone());
    assert!(mind.build_post_turn_review(&id).is_none());
}
#[test]
fn successful_outreach_action_builds_post_turn_review() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    let action = Action::outreach(
        "Check in about the deployment".into(),
        Some(ConversationId("relay:local".into())),
        Authority::Default,
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: true,
            attempted_send: true,
            attempts: 1,
            ..Outcome::default()
        },
    );

    let review = mind
        .build_post_turn_review(&id)
        .expect("successful outreach should produce review");

    assert!(matches!(review.kind, ActionKind::Review));
    assert_eq!(
        review.conversation,
        Some(ConversationId("relay:local".into()))
    );
    assert_eq!(review.authority, Authority::Default);
    assert!(review.source_messages.is_empty());
}
#[test]
fn failed_visible_action_does_not_build_post_turn_review() {
    let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
    let mut mind = test_mind(store);
    let msg = inbound(
        "relay",
        "local",
        "Sam",
        "local",
        "relay:local",
        None,
        "local-msg-1",
    );
    let action = Action::respond(
        vec![msg],
        ConversationId("relay:local".into()),
        Authority::Default,
        None,
    );
    let id = mind.registry.schedule(action);
    mind.registry.complete(
        &id,
        Outcome {
            responded: false,
            attempted_send: false,
            attempts: 1,
            ..Outcome::default()
        },
    );

    assert!(mind.build_post_turn_review(&id).is_none());
}
