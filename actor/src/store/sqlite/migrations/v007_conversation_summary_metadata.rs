use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            gateway_id TEXT,
            identity_id TEXT,
            profile_id TEXT,
            person_id TEXT,
            group_id TEXT,
            summary TEXT,
            summary_covered_message_ids TEXT NOT NULL DEFAULT '[]',
            summary_updated_at INTEGER,
            summary_version INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER NOT NULL,
            last_message_at INTEGER NOT NULL,
            message_count INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    let columns = common::table_columns(conn, "conversations")?;

    for (name, definition) in [
        ("summary_covered_message_ids", "TEXT NOT NULL DEFAULT '[]'"),
        ("summary_updated_at", "INTEGER"),
        ("summary_version", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE conversations ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    Ok(())
}
