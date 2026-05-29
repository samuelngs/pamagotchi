use super::*;
use crate::store::ChannelMembershipStatus;
use protocol::{ChannelKind, GatewayId, ProfileId, SpaceKind, channel_id, space_id};

#[tokio::test]
async fn channels_resolve_by_gateway_external_id_and_filter_by_kind() {
    let store = test_store();
    let now = 1234;
    let gateway = GatewayId("discord-main".into());
    let gateway_record = GatewayRecord {
        id: gateway.clone(),
        kind: "discord".into(),
        display_name: Some("Discord".into()),
        metadata: serde_json::json!({ "configured": true }),
        created_at: now,
        updated_at: now,
    };
    store.upsert_gateway(&gateway_record).await.unwrap();

    let space = SpaceRecord {
        id: space_id(&gateway, "guild-1"),
        gateway: gateway.clone(),
        external_id: "guild-1".into(),
        kind: SpaceKind::DiscordGuild,
        display_name: Some("Guild".into()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    };
    store.upsert_space(&space).await.unwrap();

    let channel = ChannelRecord {
        id: channel_id(&gateway, "channel-1"),
        gateway: gateway.clone(),
        external_id: "channel-1".into(),
        kind: ChannelKind::PublicChannel,
        space: Some(space.id.clone()),
        parent: None,
        display_name: Some("general".into()),
        metadata: serde_json::json!({ "topic": "ops" }),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    };
    let channel_id = store.upsert_channel(&channel).await.unwrap();

    let resolved = store
        .resolve_channel(&gateway, "channel-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.id, channel_id);
    assert_eq!(resolved.kind, ChannelKind::PublicChannel);
    assert_eq!(resolved.space, Some(space.id.clone()));

    let listed = store
        .list_channels(ChannelFilter {
            gateway: Some(gateway.clone()),
            kind: Some(ChannelKind::PublicChannel),
        })
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].external_id, "channel-1");
}

#[tokio::test]
async fn channel_memberships_store_profiles_not_people() {
    let store = test_store();
    let now = 2000;
    let gateway = GatewayId("whatsapp-main".into());
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway.clone(),
            kind: "whatsapp".into(),
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let channel = ChannelRecord {
        id: channel_id(&gateway, "family@g.us"),
        gateway: gateway.clone(),
        external_id: "family@g.us".into(),
        kind: ChannelKind::GroupChat,
        space: None,
        parent: None,
        display_name: Some("Family".into()),
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    };
    store.upsert_channel(&channel).await.unwrap();
    let profile = sample_profile("profile-alice", "Alice");
    store.add_profile(&profile).await.unwrap();

    store
        .upsert_channel_membership(&ChannelMembership {
            channel: channel.id.clone(),
            profile: ProfileId("profile-alice".into()),
            role: Some("participant".into()),
            status: ChannelMembershipStatus::Observed,
            first_seen_at: now,
            last_seen_at: now,
            metadata: serde_json::json!({ "source": "message" }),
        })
        .await
        .unwrap();

    let memberships = store.list_channel_memberships(&channel.id).await.unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].profile, ProfileId("profile-alice".into()));
    assert_eq!(memberships[0].status, ChannelMembershipStatus::Observed);
}

#[tokio::test]
async fn active_conversation_is_assigned_by_channel() {
    let store = test_store();
    let now = 3000;
    let gateway = GatewayId("relay".into());
    store
        .upsert_gateway(&GatewayRecord {
            id: gateway.clone(),
            kind: "relay".into(),
            display_name: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let channel = ChannelRecord {
        id: channel_id(&gateway, "local"),
        gateway,
        external_id: "local".into(),
        kind: ChannelKind::RelayRoom,
        space: None,
        parent: None,
        display_name: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    };
    store.upsert_channel(&channel).await.unwrap();

    let first = store
        .get_or_create_active_conversation(&channel.id, now)
        .await
        .unwrap();
    let second = store
        .get_or_create_active_conversation(&channel.id, now + 1)
        .await
        .unwrap();

    assert_eq!(first, second);
    let conn = store.lock().unwrap();
    let channel_id: String = conn
        .query_row(
            "SELECT channel_id FROM conversations WHERE id = ?1",
            [first.0.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(channel_id, channel.id.0);
}

#[tokio::test]
async fn active_conversations_are_unique_per_channel_and_resolve_back_to_channel() {
    let store = test_store();
    let now = 4000;
    let gateway = GatewayId("relay-main".into());
    store
        .upsert_gateway(&test_gateway(&gateway, "relay", now))
        .await
        .unwrap();
    let dm = test_channel(&gateway, "local", ChannelKind::Direct, now);
    let room = test_channel(&gateway, "ops", ChannelKind::RelayRoom, now);
    store.upsert_channel(&dm).await.unwrap();
    store.upsert_channel(&room).await.unwrap();

    let dm_conversation = store
        .get_or_create_active_conversation(&dm.id, now)
        .await
        .unwrap();
    let room_conversation = store
        .get_or_create_active_conversation(&room.id, now + 1)
        .await
        .unwrap();
    let dm_again = store
        .get_or_create_active_conversation(&dm.id, now + 2)
        .await
        .unwrap();

    assert_eq!(dm_conversation, dm_again);
    assert_ne!(dm_conversation, room_conversation);
    assert_eq!(
        store
            .channel_for_conversation(&dm_conversation)
            .await
            .unwrap()
            .unwrap()
            .id,
        dm.id
    );
    assert_eq!(
        store
            .channel_for_conversation(&room_conversation)
            .await
            .unwrap()
            .unwrap()
            .id,
        room.id
    );

    let conn = store.lock().unwrap();
    let active_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM conversations WHERE status = 'active' AND channel_id = ?1",
            [dm.id.0.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 1);
}

#[tokio::test]
async fn archived_conversation_allows_new_active_history_for_same_channel() {
    let store = test_store();
    let now = 5000;
    let gateway = GatewayId("discord-main".into());
    store
        .upsert_gateway(&test_gateway(&gateway, "discord", now))
        .await
        .unwrap();
    let channel = test_channel(&gateway, "channel-1", ChannelKind::PublicChannel, now);
    store.upsert_channel(&channel).await.unwrap();

    let first = store
        .get_or_create_active_conversation(&channel.id, now)
        .await
        .unwrap();
    {
        let conn = store.lock().unwrap();
        conn.execute(
            "UPDATE conversations SET status = 'archived' WHERE id = ?1",
            [first.0.as_str()],
        )
        .unwrap();
    }
    let second = store
        .get_or_create_active_conversation(&channel.id, now + 1)
        .await
        .unwrap();

    assert_ne!(first, second);
    assert_eq!(
        store
            .channel_for_conversation(&first)
            .await
            .unwrap()
            .unwrap()
            .id,
        channel.id
    );
    assert_eq!(
        store
            .channel_for_conversation(&second)
            .await
            .unwrap()
            .unwrap()
            .id,
        channel.id
    );

    let conn = store.lock().unwrap();
    let active_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM conversations WHERE status = 'active' AND channel_id = ?1",
            [channel.id.0.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 1);
}

#[tokio::test]
async fn channel_backed_messages_preserve_sender_snapshots_without_changing_channel_binding() {
    let store = test_store();
    let now = 6000;
    let gateway = GatewayId("whatsapp-main".into());
    store
        .upsert_gateway(&test_gateway(&gateway, "whatsapp", now))
        .await
        .unwrap();
    let channel = test_channel(&gateway, "family@g.us", ChannelKind::GroupChat, now);
    store.upsert_channel(&channel).await.unwrap();
    let conversation = store
        .get_or_create_active_conversation(&channel.id, now)
        .await
        .unwrap();

    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now,
                role: MessageRole::User,
                content: "alice".into(),
                identity: None,
                profile: Some(ProfileId("profile-alice".into())),
                person: Some(PersonId("person-alice".into())),
                source_gateway_id: Some(gateway.0.clone()),
                source_message_id: Some("msg-a".into()),
                sender_external_id: Some("alice@s.whatsapp.net".into()),
                reply_external_id: Some("family@g.us".into()),
                metadata: serde_json::json!({ "message_id": "msg-a" }),
            },
        )
        .await
        .unwrap();
    store
        .append_message(
            &conversation,
            &StoredMessage {
                timestamp: now + 1,
                role: MessageRole::User,
                content: "bob".into(),
                identity: None,
                profile: Some(ProfileId("profile-bob".into())),
                person: Some(PersonId("person-bob".into())),
                source_gateway_id: Some(gateway.0.clone()),
                source_message_id: Some("msg-b".into()),
                sender_external_id: Some("bob@s.whatsapp.net".into()),
                reply_external_id: Some("family@g.us".into()),
                metadata: serde_json::json!({ "message_id": "msg-b" }),
            },
        )
        .await
        .unwrap();

    let messages = store.get_messages(&conversation, 10, None).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].profile, Some(ProfileId("profile-alice".into())));
    assert_eq!(messages[1].profile, Some(ProfileId("profile-bob".into())));
    assert_eq!(
        messages[0].sender_external_id.as_deref(),
        Some("alice@s.whatsapp.net")
    );
    assert_eq!(
        messages[1].sender_external_id.as_deref(),
        Some("bob@s.whatsapp.net")
    );
    assert_eq!(
        store
            .channel_for_conversation(&conversation)
            .await
            .unwrap()
            .unwrap()
            .id,
        channel.id
    );
}

fn test_gateway(gateway: &GatewayId, kind: &str, now: i64) -> GatewayRecord {
    GatewayRecord {
        id: gateway.clone(),
        kind: kind.into(),
        display_name: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

fn test_channel(
    gateway: &GatewayId,
    external_id: &str,
    kind: ChannelKind,
    now: i64,
) -> ChannelRecord {
    ChannelRecord {
        id: channel_id(gateway, external_id),
        gateway: gateway.clone(),
        external_id: external_id.into(),
        kind,
        space: None,
        parent: None,
        display_name: None,
        metadata: serde_json::json!({}),
        created_at: now,
        updated_at: now,
        last_seen_at: now,
    }
}
