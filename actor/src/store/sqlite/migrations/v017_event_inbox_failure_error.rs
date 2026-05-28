use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_inbox (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            due_at INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            dedupe_key TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            fired_at INTEGER,
            last_error TEXT
        );",
    )?;

    let columns = common::table_columns(conn, "event_inbox")?;
    if !columns.contains("last_error") {
        conn.execute("ALTER TABLE event_inbox ADD COLUMN last_error TEXT", [])?;
    }
    Ok(())
}
