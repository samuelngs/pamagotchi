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
        );
        CREATE INDEX IF NOT EXISTS idx_event_inbox_due
            ON event_inbox(status, due_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_inbox_dedupe
            ON event_inbox(dedupe_key)
            WHERE dedupe_key IS NOT NULL AND status = 'pending';",
    )?;
    Ok(())
}
