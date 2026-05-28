use super::support::SlowSqliteQuery;
use crate::store::EventInboxRecord;
use rusqlite::{Connection, Row, params};
use std::collections::HashSet;

pub(super) fn enqueue_event(conn: &Connection, event: &EventInboxRecord) -> anyhow::Result<()> {
    let payload_json = serde_json::to_string(&event.payload)?;
    conn.execute(
        "INSERT OR IGNORE INTO event_inbox (
            id, kind, payload_json, status, due_at, attempts, dedupe_key,
            created_at, updated_at, fired_at, last_error
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            event.id.as_str(),
            event.kind.as_str(),
            payload_json,
            event.status.as_str(),
            event.due_at,
            event.attempts,
            event.dedupe_key.as_deref(),
            event.created_at,
            event.updated_at,
            event.fired_at,
            event.last_error.as_deref(),
        ],
    )?;
    Ok(())
}

pub(super) fn pending_events_by_kind(
    conn: &Connection,
    kind: &str,
    limit: usize,
) -> anyhow::Result<Vec<EventInboxRecord>> {
    let _slow_query = SlowSqliteQuery::start("pending_events_by_kind");
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload_json, status, due_at, attempts, dedupe_key,
                created_at, updated_at, fired_at, last_error
         FROM event_inbox
         WHERE status = 'pending' AND kind = ?1
         ORDER BY due_at ASC, created_at ASC
         LIMIT ?2",
    )?;
    read_events(&mut stmt, params![kind, limit as i64])
}

pub(super) fn due_events(
    conn: &Connection,
    now: i64,
    limit: usize,
) -> anyhow::Result<Vec<EventInboxRecord>> {
    let _slow_query = SlowSqliteQuery::start("due_events");
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload_json, status, due_at, attempts, dedupe_key,
                created_at, updated_at, fired_at, last_error
         FROM event_inbox
         WHERE status = 'pending' AND due_at <= ?1
         ORDER BY due_at ASC, created_at ASC
         LIMIT ?2",
    )?;
    let candidate_limit = limit.saturating_mul(4).max(limit);
    let candidates = read_events(&mut stmt, params![now, candidate_limit as i64])?;
    Ok(coalesce_due_events(candidates, limit))
}

pub(super) fn mark_event_fired(conn: &Connection, id: &str, fired_at: i64) -> anyhow::Result<bool> {
    let rows = conn.execute(
        "UPDATE event_inbox
         SET status = 'fired', attempts = attempts + 1, updated_at = ?2, fired_at = ?2
         WHERE id = ?1 AND status = 'pending'",
        params![id, fired_at],
    )?;
    Ok(rows > 0)
}

pub(super) fn mark_event_failed(
    conn: &Connection,
    id: &str,
    failed_at: i64,
    error: Option<&str>,
) -> anyhow::Result<bool> {
    let rows = conn.execute(
        "UPDATE event_inbox
         SET status = 'failed', attempts = attempts + 1, updated_at = ?2, last_error = ?3
         WHERE id = ?1 AND status = 'pending'",
        params![id, failed_at, error],
    )?;
    Ok(rows > 0)
}

fn read_events<P>(
    stmt: &mut rusqlite::Statement<'_>,
    params: P,
) -> anyhow::Result<Vec<EventInboxRecord>>
where
    P: rusqlite::Params,
{
    let mut rows = stmt.query(params)?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        events.push(read_event(row)?);
    }
    Ok(events)
}

fn read_event(row: &Row<'_>) -> rusqlite::Result<EventInboxRecord> {
    let payload_json: String = row.get("payload_json")?;
    let mut last_error: Option<String> = row.get("last_error")?;
    let payload = match serde_json::from_str(&payload_json) {
        Ok(payload) => payload,
        Err(e) => {
            let parse_error = format!("malformed event payload json: {e}");
            last_error = match last_error {
                Some(existing) => Some(format!("{existing}; {parse_error}")),
                None => Some(parse_error),
            };
            serde_json::Value::Null
        }
    };

    Ok(EventInboxRecord {
        id: row.get("id")?,
        kind: row.get("kind")?,
        payload,
        status: row.get("status")?,
        due_at: row.get("due_at")?,
        attempts: row.get("attempts")?,
        dedupe_key: row.get("dedupe_key")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        fired_at: row.get("fired_at")?,
        last_error,
    })
}

fn coalesce_due_events(candidates: Vec<EventInboxRecord>, limit: usize) -> Vec<EventInboxRecord> {
    let mut seen_message_conversations = HashSet::new();
    let mut events = Vec::new();
    for event in candidates {
        if let Some(conversation) = message_conversation_key(&event) {
            if !seen_message_conversations.insert(conversation) {
                continue;
            }
        }

        events.push(event);
        if events.len() >= limit {
            break;
        }
    }
    events
}

fn message_conversation_key(event: &EventInboxRecord) -> Option<String> {
    if event.kind != "message" {
        return None;
    }
    event
        .payload
        .get("conversation")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}
