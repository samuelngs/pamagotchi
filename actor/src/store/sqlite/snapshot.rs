use crate::store::{ActorSnapshot, StateJournalRecord};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn save_snapshot(conn: &Connection, snapshot: &ActorSnapshot) -> anyhow::Result<()> {
    let data = serde_json::to_string(snapshot)?;
    conn.execute(
        "INSERT INTO snapshots (saved_at, data) VALUES (?1, ?2)",
        params![snapshot.saved_at, data],
    )?;
    Ok(())
}

pub(super) fn load_latest_snapshot(conn: &Connection) -> anyhow::Result<Option<ActorSnapshot>> {
    let data = conn
        .query_row(
            "SELECT data FROM snapshots ORDER BY saved_at DESC, id DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    data.map(|data| serde_json::from_str(&data).map_err(Into::into))
        .transpose()
}

pub(super) fn append_state_journal(
    conn: &Connection,
    kind: &str,
    payload: &serde_json::Value,
    created_at: i64,
) -> anyhow::Result<i64> {
    let payload_json = serde_json::to_string(payload)?;
    conn.execute(
        "INSERT INTO state_journal (kind, payload_json, created_at)
         VALUES (?1, ?2, ?3)",
        params![kind, payload_json, created_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub(super) fn state_journal_after(
    conn: &Connection,
    after_id: Option<i64>,
    limit: usize,
) -> anyhow::Result<Vec<StateJournalRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload_json, created_at
         FROM state_journal
         WHERE id > ?1
         ORDER BY id ASC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![after_id.unwrap_or(0), limit as i64], |row| {
            let payload_json: String = row.get(2)?;
            let payload = serde_json::from_str(&payload_json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(StateJournalRecord {
                id: row.get(0)?,
                kind: row.get(1)?,
                payload,
                created_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
