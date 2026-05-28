use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_runs (
            action_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            task TEXT NOT NULL,
            conversation_id TEXT,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            status TEXT NOT NULL,
            responded INTEGER NOT NULL DEFAULT 0,
            attempts INTEGER NOT NULL DEFAULT 0,
            memories_formed TEXT NOT NULL DEFAULT '[]',
            recalled_memory_ids TEXT NOT NULL DEFAULT '[]'
        );",
    )?;

    let columns = common::table_columns(conn, "action_runs")?;

    for (name, definition) in [
        ("memories_formed", "TEXT NOT NULL DEFAULT '[]'"),
        ("recalled_memory_ids", "TEXT NOT NULL DEFAULT '[]'"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE action_runs ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    Ok(())
}
