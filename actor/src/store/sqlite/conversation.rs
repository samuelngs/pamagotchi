use super::rows::read_message;
use super::support::{SlowSqliteQuery, TxGuard, message_revision_metadata};
use crate::store::{ConversationSummary, MessageRole, StoredMessage};
use protocol::{
    ChannelId, ChannelKind, ConversationId, GatewayId, GroupId, IdentityId, MessageDirection,
    MessageId, PersonId, ProfileId, channel_id, generated_message_id, inbound_message_id,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

pub(super) fn append_message(
    conn: &Connection,
    conv: &ConversationId,
    msg: &StoredMessage,
) -> anyhow::Result<()> {
    let _slow_query = SlowSqliteQuery::start("append_message");
    let tx = TxGuard::begin(conn)?;
    let identity_id = msg.identity.as_ref().map(|p| &p.0);
    let profile_id = msg.profile.as_ref().map(|p| &p.0);
    let person_id = msg.person.as_ref().map(|p| &p.0);
    let source_gateway_id = msg.source_gateway_id.as_deref();
    let source_message_id = msg.source_message_id.as_deref();
    let sender_external_id = msg.sender_external_id.as_deref();
    let reply_external_id = msg.reply_external_id.as_deref();
    let channel_id = ensure_conversation_channel(conn, conv, msg)?;
    let message_id = stored_message_id(&channel_id, msg);
    let direction = message_direction(&msg.role);

    conn.execute(
        "INSERT INTO conversations (id, channel_id, started_at, last_message_at, message_count)
         VALUES (?1, ?2, ?3, ?3, 0)
         ON CONFLICT(id) DO UPDATE SET
            channel_id = conversations.channel_id",
        params![conv.0, channel_id.0, msg.timestamp],
    )?;

    let metadata_json = serde_json::to_string(&msg.metadata)?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO messages (
            message_id,
            conversation_id,
            channel_id,
            direction,
            timestamp,
            role,
            content,
            identity_id,
            profile_id,
            person_id,
            source_gateway_id,
            source_message_id,
            sender_external_id,
            reply_external_id,
            metadata
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            message_id.0,
            conv.0,
            channel_id.0,
            direction,
            msg.timestamp,
            msg.role.as_str(),
            msg.content,
            identity_id,
            profile_id,
            person_id,
            source_gateway_id,
            source_message_id,
            sender_external_id,
            reply_external_id,
            metadata_json,
        ],
    )?;

    if inserted == 0 {
        tx.commit()?;
        return Ok(());
    }

    conn.execute(
        "UPDATE conversations SET
            last_message_at = CASE WHEN last_message_at < ?2 THEN ?2 ELSE last_message_at END,
            message_count = message_count + 1
         WHERE id = ?1",
        params![conv.0, msg.timestamp],
    )?;

    if let Some(profile) = &msg.profile {
        conn.execute(
            "UPDATE profiles SET last_seen = ?1, updated_at = ?1 WHERE id = ?2 AND last_seen < ?1",
            params![msg.timestamp, profile.0],
        )?;
    }
    if let Some(person) = &msg.person {
        conn.execute(
            "UPDATE persons SET updated_at = ?1 WHERE id = ?2 AND updated_at < ?1",
            params![msg.timestamp, person.0],
        )?;
    }

    tx.commit()?;
    Ok(())
}

fn ensure_conversation_channel(
    conn: &Connection,
    conv: &ConversationId,
    msg: &StoredMessage,
) -> anyhow::Result<ChannelId> {
    if let Some(existing) = conn
        .query_row(
            "SELECT channel_id FROM conversations WHERE id = ?1",
            params![conv.0],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(ChannelId(existing));
    }

    if let Some(channel) = metadata_channel_id(msg) {
        if conn
            .query_row(
                "SELECT 1 FROM channels WHERE id = ?1",
                params![channel.0],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(channel);
        }
    }

    let source_gateway = msg
        .source_gateway_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let reply_target = msg
        .reply_external_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let synthetic_channel = source_gateway.is_none() || reply_target.is_none();
    let gateway = source_gateway.unwrap_or("local");
    let external_id = reply_target.unwrap_or(conv.0.as_str());
    let gateway_id = GatewayId(gateway.to_string());
    let channel = metadata_channel_id(msg).unwrap_or_else(|| channel_id(&gateway_id, external_id));
    let channel_metadata = if synthetic_channel {
        serde_json::json!({
            "synthetic": true,
            "delivery_supported": false,
        })
    } else {
        serde_json::json!({})
    };
    let channel_metadata_json = serde_json::to_string(&channel_metadata)?;

    conn.execute(
        "INSERT INTO gateways (id, kind, display_name, metadata_json, created_at, updated_at)
         VALUES (?1, ?2, NULL, '{}', ?3, ?3)
         ON CONFLICT(id) DO NOTHING",
        params![gateway_id.0, gateway, msg.timestamp],
    )?;
    conn.execute(
        "INSERT INTO channels (
            id, gateway_id, external_id, kind, metadata_json,
            created_at, updated_at, last_seen_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6)
         ON CONFLICT(gateway_id, external_id) DO NOTHING",
        params![
            channel.0,
            gateway_id.0,
            external_id,
            ChannelKind::Unknown.as_str(),
            channel_metadata_json,
            msg.timestamp,
        ],
    )?;
    Ok(channel)
}

fn metadata_channel_id(msg: &StoredMessage) -> Option<ChannelId> {
    msg.metadata
        .get("channel_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| ChannelId(id.to_string()))
}

fn stored_message_id(channel: &ChannelId, msg: &StoredMessage) -> MessageId {
    if let Some(id) = msg
        .metadata
        .get("message_id")
        .and_then(|value| value.as_str())
        .filter(|id| !id.trim().is_empty())
        .filter(|id| id.starts_with("message:") || id.starts_with("local:"))
    {
        return MessageId(id.to_string());
    }
    if matches!(msg.role, MessageRole::User) {
        if let Some(source) = msg
            .source_message_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
        {
            return inbound_message_id(channel, source);
        }
    }
    if msg.source_message_id.is_some() {
        MessageId(msg.readable_message_id())
    } else {
        generated_message_id()
    }
}

fn message_direction(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => MessageDirection::Inbound.as_str(),
        MessageRole::Assistant => MessageDirection::Outbound.as_str(),
        MessageRole::System | MessageRole::Tool => MessageDirection::Internal.as_str(),
    }
}

pub(super) fn update_message_content_by_source(
    conn: &Connection,
    conv: &ConversationId,
    gateway_id: &str,
    source_message_id: &str,
    content: &str,
    edited_at: i64,
) -> anyhow::Result<bool> {
    let metadata_json: Option<String> = conn
        .query_row(
            "SELECT metadata
             FROM messages
             WHERE conversation_id = ?1
                AND source_gateway_id = ?2
                AND source_message_id = ?3
                AND role = 'user'
             LIMIT 1",
            params![conv.0, gateway_id, source_message_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(metadata_json) = metadata_json else {
        return Ok(false);
    };

    let metadata_json = message_revision_metadata(metadata_json, |metadata| {
        metadata.insert("edited".into(), serde_json::json!(true));
        metadata.insert("edited_at".into(), serde_json::json!(edited_at));
        metadata.remove("deleted");
        metadata.remove("deleted_at");
    })?;
    let tx = TxGuard::begin(conn)?;
    let rows = conn.execute(
        "UPDATE messages
         SET content = ?4, metadata = ?5
         WHERE conversation_id = ?1
            AND source_gateway_id = ?2
            AND source_message_id = ?3
            AND role = 'user'",
        params![
            conv.0,
            gateway_id,
            source_message_id,
            content,
            metadata_json
        ],
    )?;
    conn.execute(
        "UPDATE action_messages
         SET content = ?3
         WHERE role = 'user'
            AND source_gateway_id = ?1
            AND source_message_id = ?2",
        params![gateway_id, source_message_id, content],
    )?;
    tx.commit()?;
    Ok(rows > 0)
}

pub(super) fn mark_message_deleted_by_source(
    conn: &Connection,
    conv: &ConversationId,
    gateway_id: &str,
    source_message_id: &str,
    deleted_at: i64,
) -> anyhow::Result<bool> {
    let metadata_json: Option<String> = conn
        .query_row(
            "SELECT metadata
             FROM messages
             WHERE conversation_id = ?1
                AND source_gateway_id = ?2
                AND source_message_id = ?3
                AND role = 'user'
             LIMIT 1",
            params![conv.0, gateway_id, source_message_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(metadata_json) = metadata_json else {
        return Ok(false);
    };

    let metadata_json = message_revision_metadata(metadata_json, |metadata| {
        metadata.insert("deleted".into(), serde_json::json!(true));
        metadata.insert("deleted_at".into(), serde_json::json!(deleted_at));
    })?;
    let tx = TxGuard::begin(conn)?;
    let rows = conn.execute(
        "UPDATE messages
         SET content = '[message deleted]', metadata = ?4
         WHERE conversation_id = ?1
            AND source_gateway_id = ?2
            AND source_message_id = ?3
            AND role = 'user'",
        params![conv.0, gateway_id, source_message_id, metadata_json],
    )?;
    conn.execute(
        "UPDATE action_messages
         SET content = '[message deleted]'
         WHERE role = 'user'
            AND source_gateway_id = ?1
            AND source_message_id = ?2",
        params![gateway_id, source_message_id],
    )?;
    tx.commit()?;
    Ok(rows > 0)
}

pub(super) fn get_messages(
    conn: &Connection,
    conv: &ConversationId,
    limit: usize,
    before: Option<i64>,
) -> anyhow::Result<Vec<StoredMessage>> {
    let _slow_query = SlowSqliteQuery::start("get_messages");

    let mut messages = if let Some(before_ts) = before {
        let mut stmt = conn.prepare(
            "SELECT timestamp, role, content, identity_id, profile_id, person_id,
                    source_gateway_id, source_message_id, sender_external_id, reply_external_id, metadata
             FROM messages
             WHERE conversation_id = ?1 AND timestamp < ?2
             ORDER BY timestamp DESC, id DESC
             LIMIT ?3",
        )?;
        stmt.query_map(params![conv.0, before_ts, limit as i64], read_message)?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
    } else {
        let mut stmt = conn.prepare(
            "SELECT timestamp, role, content, identity_id, profile_id, person_id,
                    source_gateway_id, source_message_id, sender_external_id, reply_external_id, metadata
             FROM messages
             WHERE conversation_id = ?1
             ORDER BY timestamp DESC, id DESC
             LIMIT ?2",
        )?;
        stmt.query_map(params![conv.0, limit as i64], read_message)?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
    };

    messages.reverse();
    Ok(messages)
}

pub(super) fn list_conversations(conn: &Connection) -> anyhow::Result<Vec<ConversationSummary>> {
    let _slow_query = SlowSqliteQuery::start("list_conversations");
    let mut stmt = conn.prepare(
        "SELECT
                c.id,
                COALESCE(
                    ch.gateway_id,
                    (
                        SELECT m.source_gateway_id
                        FROM messages m
                        WHERE m.conversation_id = c.id
                          AND m.source_gateway_id IS NOT NULL
                        ORDER BY m.timestamp DESC, m.id DESC
                        LIMIT 1
                    )
                ) AS gateway_id,
                (
                    SELECT m.identity_id
                    FROM messages m
                    WHERE m.conversation_id = c.id
                      AND m.identity_id IS NOT NULL
                    ORDER BY m.timestamp DESC, m.id DESC
                    LIMIT 1
                ) AS identity_id,
                (
                    SELECT m.profile_id
                    FROM messages m
                    WHERE m.conversation_id = c.id
                      AND m.profile_id IS NOT NULL
                    ORDER BY m.timestamp DESC, m.id DESC
                    LIMIT 1
                ) AS profile_id,
                (
                    SELECT m.person_id
                    FROM messages m
                    WHERE m.conversation_id = c.id
                      AND m.person_id IS NOT NULL
                    ORDER BY m.timestamp DESC, m.id DESC
                    LIMIT 1
                ) AS person_id,
                ch.kind AS channel_kind,
                ch.external_id AS channel_external_id,
                ch.metadata_json AS channel_metadata_json,
                c.summary,
                c.summary_covered_message_ids,
                c.summary_updated_at,
                c.summary_version,
                c.message_count,
                c.started_at,
                c.last_message_at
         FROM conversations c
         LEFT JOIN channels ch ON ch.id = c.channel_id
         ORDER BY c.last_message_at DESC",
    )?;
    let results = stmt
        .query_map([], |row| {
            let identity_id: Option<String> = row.get("identity_id")?;
            let profile_id: Option<String> = row.get("profile_id")?;
            let person_id: Option<String> = row.get("person_id")?;
            let channel_kind: Option<String> = row.get("channel_kind")?;
            let channel_external_id: Option<String> = row.get("channel_external_id")?;
            let channel_metadata_json: Option<String> = row.get("channel_metadata_json")?;
            let covered_json: String = row.get("summary_covered_message_ids")?;
            Ok(ConversationSummary {
                id: ConversationId(row.get("id")?),
                gateway_id: row.get("gateway_id")?,
                identity: identity_id.map(IdentityId),
                profile: profile_id.map(ProfileId),
                person: person_id.map(PersonId),
                group: legacy_group_from_channel(
                    channel_kind.as_deref(),
                    channel_external_id.as_deref(),
                    channel_metadata_json.as_deref(),
                ),
                summary: row.get("summary")?,
                summary_covered_message_ids: serde_json::from_str(&covered_json)
                    .unwrap_or_default(),
                summary_updated_at: row.get("summary_updated_at")?,
                summary_version: row.get("summary_version")?,
                message_count: row.get("message_count")?,
                started_at: row.get("started_at")?,
                last_message_at: row.get("last_message_at")?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

fn legacy_group_from_channel(
    channel_kind: Option<&str>,
    channel_external_id: Option<&str>,
    channel_metadata_json: Option<&str>,
) -> Option<GroupId> {
    let metadata = channel_metadata_json
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .unwrap_or_default();
    metadata
        .get("group_id")
        .or_else(|| metadata.get("guild_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| GroupId(id.to_string()))
        .or_else(|| {
            matches!(channel_kind, Some("group_chat"))
                .then(|| channel_external_id.map(|id| GroupId(id.to_string())))
                .flatten()
        })
}

pub(super) fn update_conversation_summary(
    conn: &Connection,
    conv: &ConversationId,
    summary: &str,
    covered_message_ids: &[String],
) -> anyhow::Result<()> {
    let existing_covered_json: Option<String> = conn
        .query_row(
            "SELECT summary_covered_message_ids FROM conversations WHERE id = ?1",
            params![conv.0],
            |row| row.get(0),
        )
        .optional()?;
    let covered_message_ids =
        merge_covered_message_ids(existing_covered_json.as_deref(), covered_message_ids);
    let covered_json = serde_json::to_string(&covered_message_ids)?;
    let tx = TxGuard::begin(conn)?;
    let updated = conn.execute(
        "UPDATE conversations SET
            summary = ?1,
            summary_covered_message_ids = ?2,
            summary_updated_at = unixepoch(),
            summary_version = summary_version + 1
         WHERE id = ?3",
        params![summary, covered_json, conv.0],
    )?;
    if updated > 0 {
        let summary_version: i64 = conn.query_row(
            "SELECT summary_version FROM conversations WHERE id = ?1",
            params![conv.0],
            |row| row.get(0),
        )?;
        for message_id in &covered_message_ids {
            conn.execute(
                "INSERT OR IGNORE INTO conversation_summary_coverage (
                    conversation_id, summary_version, message_id
                 )
                 VALUES (?1, ?2, ?3)",
                params![conv.0, summary_version, message_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn merge_covered_message_ids(existing_json: Option<&str>, proposed: &[String]) -> Vec<String> {
    let mut merged = existing_json
        .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .unwrap_or_default();
    let mut seen = merged.iter().cloned().collect::<HashSet<_>>();
    for id in proposed
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
    {
        if seen.insert(id.to_string()) {
            merged.push(id.to_string());
        }
    }
    merged
}
