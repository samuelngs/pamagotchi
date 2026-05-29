use super::*;

#[tokio::test]
async fn conversation_messages() {
    let store = test_store();
    let conv = ConversationId("c1".into());

    store
        .append_message(
            &conv,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "hello".into(),
                identity: None,
                profile: Some(ProfileId("profile-sam".into())),
                person: Some(PersonId("sam".into())),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    store
        .append_message(
            &conv,
            &StoredMessage {
                timestamp: 1001,
                role: MessageRole::Assistant,
                content: "hi there".into(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some("local".into()),
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();

    let msgs = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "hello");
    assert_eq!(msgs[1].content, "hi there");
    let expected_channel = protocol::channel_id(&GatewayId("relay".into()), "local");
    {
        let conn = store.lock().unwrap();
        let stored_rows = conn
            .prepare(
                "SELECT message_id, channel_id, direction
                 FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY timestamp, id",
            )
            .unwrap()
            .query_map(params![conv.0.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(stored_rows.len(), 2);
        assert!(stored_rows[0].0.starts_with("message:"));
        assert_eq!(stored_rows[0].1, expected_channel.0);
        assert_eq!(stored_rows[0].2, "inbound");
        assert_eq!(stored_rows[1].1, expected_channel.0);
        assert_eq!(stored_rows[1].2, "outbound");
    }

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].message_count, 2);
    assert_eq!(convs[0].summary_version, 0);

    store
        .update_conversation_summary(
            &conv,
            "Sam said hello and the actor replied.",
            &[String::from("msg-1")],
        )
        .await
        .unwrap();

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(
        convs[0].summary.as_deref(),
        Some("Sam said hello and the actor replied.")
    );
    assert_eq!(
        convs[0].summary_covered_message_ids,
        vec![String::from("msg-1")]
    );
    assert!(convs[0].summary_updated_at.is_some());
    assert_eq!(convs[0].summary_version, 1);

    store
        .update_conversation_summary(
            &conv,
            "Sam said hello; the actor replied and the next message is covered.",
            &[String::from("msg-2"), String::from("msg-1")],
        )
        .await
        .unwrap();

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(
        convs[0].summary_covered_message_ids,
        vec![String::from("msg-1"), String::from("msg-2")]
    );
    assert_eq!(convs[0].summary_version, 2);

    let conn = store.lock().unwrap();
    let coverage = conn
        .prepare(
            "SELECT summary_version, message_id
             FROM conversation_summary_coverage
             WHERE conversation_id = ?1
             ORDER BY summary_version, message_id",
        )
        .unwrap()
        .query_map(params![conv.0.as_str()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        coverage,
        vec![
            (1, String::from("msg-1")),
            (2, String::from("msg-1")),
            (2, String::from("msg-2")),
        ]
    );
}

#[tokio::test]
async fn inbound_message_append_is_idempotent_and_derives_group_context_from_channel() {
    let store = test_store();
    let group = GroupId("discord:guild-1".into());
    let gateway = GatewayId("discord".into());
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway.clone(),
            kind: "discord".into(),
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: 1000,
            updated_at: 1000,
        })
        .await
        .unwrap();
    let channel = ChannelRecord {
        id: protocol::channel_id(&gateway, "channel-1"),
        gateway: gateway.clone(),
        external_id: "channel-1".into(),
        kind: protocol::ChannelKind::PublicChannel,
        space: None,
        parent: None,
        display_name: None,
        metadata: serde_json::json!({
            "group_id": group.0.clone(),
        }),
        created_at: 1000,
        updated_at: 1000,
        last_seen_at: 1000,
    };
    store.upsert_channel(&channel).await.unwrap();
    let conv = store
        .get_or_create_active_conversation(&channel.id, 1000)
        .await
        .unwrap();
    let msg = StoredMessage {
        timestamp: 1000,
        role: MessageRole::User,
        content: "hello group".into(),
        identity: Some(IdentityId("identity-a".into())),
        profile: Some(ProfileId("profile-a".into())),
        person: Some(PersonId("person-a".into())),
        source_gateway_id: Some("discord".into()),
        source_message_id: Some("discord-msg-1".into()),
        sender_external_id: Some("author-a".into()),
        reply_external_id: Some("channel-1".into()),
        metadata: serde_json::json!({"message_id": "discord-msg-1"}),
    };

    store.append_message(&conv, &msg).await.unwrap();
    store.append_message(&conv, &msg).await.unwrap();

    let msgs = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].source_message_id.as_deref(), Some("discord-msg-1"));
    assert_eq!(msgs[0].sender_external_id.as_deref(), Some("author-a"));
    assert_eq!(msgs[0].reply_external_id.as_deref(), Some("channel-1"));

    let convs = store.list_conversations().await.unwrap();
    assert_eq!(convs[0].message_count, 1);
    assert_eq!(convs[0].gateway_id.as_deref(), Some("discord"));
    assert_eq!(convs[0].group.as_ref(), Some(&group));
}

#[tokio::test]
async fn message_edit_and_delete_update_visible_history_and_action_sources() {
    let store = test_store();
    let conv = ConversationId("discord:channel-1".into());

    store
        .append_message(
            &conv,
            &StoredMessage {
                timestamp: 1000,
                role: MessageRole::User,
                content: "before edit".into(),
                identity: None,
                profile: Some(ProfileId("profile-a".into())),
                person: Some(PersonId("person-a".into())),
                source_gateway_id: Some("discord".into()),
                source_message_id: Some("discord-msg-1".into()),
                sender_external_id: Some("author-a".into()),
                reply_external_id: Some("channel-1".into()),
                metadata: serde_json::json!({"message_id": "discord-msg-1"}),
            },
        )
        .await
        .unwrap();
    store
        .append_action_message(&ActionMessageRecord {
            action_id: "action-1".into(),
            role: "user".into(),
            conversation: Some(conv.clone()),
            source_gateway_id: Some("discord".into()),
            source_message_id: Some("discord-msg-1".into()),
            sender_external_id: Some("author-a".into()),
            reply_external_id: Some("channel-1".into()),
            content: Some("before edit".into()),
            created_at: 1000,
        })
        .await
        .unwrap();

    assert!(
        store
            .update_message_content_by_source(
                &conv,
                "discord",
                "discord-msg-1",
                "after edit",
                1100,
            )
            .await
            .unwrap()
    );
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "after edit");
    assert_eq!(messages[0].metadata["edited"], true);
    assert_eq!(messages[0].metadata["edited_at"], 1100);
    let transcript = store.action_transcript("action-1").await.unwrap();
    assert_eq!(
        transcript.messages[0].content.as_deref(),
        Some("after edit")
    );

    assert!(
        store
            .mark_message_deleted_by_source(&conv, "discord", "discord-msg-1", 1200)
            .await
            .unwrap()
    );
    let messages = store.get_messages(&conv, 10, None).await.unwrap();
    assert_eq!(messages[0].content, "[message deleted]");
    assert_eq!(messages[0].metadata["deleted"], true);
    assert_eq!(messages[0].metadata["deleted_at"], 1200);
    let transcript = store.action_transcript("action-1").await.unwrap();
    assert_eq!(
        transcript.messages[0].content.as_deref(),
        Some("[message deleted]")
    );
}
