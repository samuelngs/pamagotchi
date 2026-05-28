use super::rows::read_message;
use super::support::{SlowSqliteQuery, TxGuard, message_revision_metadata};
use crate::store::{ConversationSummary, StoredMessage};
use protocol::{ConversationId, GroupId, IdentityId, PersonId, ProfileId};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

pub(super) fn append_message(
    conn: &Connection,
    conv: &ConversationId,
    gateway_id: Option<&str>,
    group: Option<&GroupId>,
    msg: &StoredMessage,
) -> anyhow::Result<()> {
    let _slow_query = SlowSqliteQuery::start("append_message");
    let tx = TxGuard::begin(conn)?;
    let identity_id = msg.identity.as_ref().map(|p| &p.0);
    let profile_id = msg.profile.as_ref().map(|p| &p.0);
    let person_id = msg.person.as_ref().map(|p| &p.0);
    let group_id = group.map(|g| &g.0);
    let source_gateway_id = msg.source_gateway_id.as_deref();
    let source_message_id = msg.source_message_id.as_deref();
    let sender_external_id = msg.sender_external_id.as_deref();
    let reply_external_id = msg.reply_external_id.as_deref();

    conn.execute(
        "INSERT INTO conversations (id, gateway_id, identity_id, profile_id, person_id, group_id, started_at, last_message_at, message_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 0)
         ON CONFLICT(id) DO UPDATE SET
            gateway_id = COALESCE(conversations.gateway_id, excluded.gateway_id),
            identity_id = COALESCE(excluded.identity_id, conversations.identity_id),
            profile_id = COALESCE(excluded.profile_id, conversations.profile_id),
            person_id = COALESCE(conversations.person_id, excluded.person_id),
            group_id = COALESCE(conversations.group_id, excluded.group_id)",
        params![
            conv.0,
            gateway_id,
            identity_id,
            profile_id,
            person_id,
            group_id,
            msg.timestamp,
        ],
    )?;

    let metadata_json = serde_json::to_string(&msg.metadata)?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO messages (
            conversation_id,
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
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            conv.0,
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
            message_count = message_count + 1,
            gateway_id = COALESCE(gateway_id, ?3),
            identity_id = COALESCE(?4, identity_id),
            profile_id = COALESCE(?5, profile_id),
            person_id = COALESCE(person_id, ?6),
            group_id = COALESCE(group_id, ?7)
         WHERE id = ?1",
        params![
            conv.0,
            msg.timestamp,
            gateway_id,
            identity_id,
            profile_id,
            person_id,
            group_id,
        ],
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
        "SELECT id, gateway_id, identity_id, profile_id, person_id, group_id, summary,
                summary_covered_message_ids, summary_updated_at, summary_version,
                message_count, started_at, last_message_at
         FROM conversations ORDER BY last_message_at DESC",
    )?;
    let results = stmt
        .query_map([], |row| {
            let identity_id: Option<String> = row.get("identity_id")?;
            let profile_id: Option<String> = row.get("profile_id")?;
            let person_id: Option<String> = row.get("person_id")?;
            let group_id: Option<String> = row.get("group_id")?;
            let covered_json: String = row.get("summary_covered_message_ids")?;
            Ok(ConversationSummary {
                id: ConversationId(row.get("id")?),
                gateway_id: row.get("gateway_id")?,
                identity: identity_id.map(IdentityId),
                profile: profile_id.map(ProfileId),
                person: person_id.map(PersonId),
                group: group_id.map(GroupId),
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
    conn.execute(
        "UPDATE conversations SET
            summary = ?1,
            summary_covered_message_ids = ?2,
            summary_updated_at = unixepoch(),
            summary_version = summary_version + 1
         WHERE id = ?3",
        params![summary, covered_json, conv.0],
    )?;
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
