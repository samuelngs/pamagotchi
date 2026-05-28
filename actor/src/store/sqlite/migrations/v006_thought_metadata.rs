use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS thoughts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            memories_accessed TEXT NOT NULL DEFAULT '[]',
            subjects TEXT NOT NULL DEFAULT '[]'
        );",
    )?;

    let columns = common::table_columns(conn, "thoughts")?;

    for (name, definition) in [
        ("importance", "REAL NOT NULL DEFAULT 0.5"),
        ("confidence", "REAL NOT NULL DEFAULT 0.5"),
        ("action_id", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE thoughts ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_thoughts_ts ON thoughts(timestamp);
         CREATE INDEX IF NOT EXISTS idx_thoughts_signal ON thoughts(importance, confidence, timestamp);
         CREATE INDEX IF NOT EXISTS idx_thoughts_action ON thoughts(action_id);",
    )?;
    Ok(())
}
