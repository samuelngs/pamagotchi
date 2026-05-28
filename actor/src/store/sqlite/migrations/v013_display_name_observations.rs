use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS display_name_observations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            identity_id TEXT NOT NULL,
            profile_id TEXT,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            display_name TEXT NOT NULL,
            source_message_id TEXT,
            observed_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_display_name_observations_identity
            ON display_name_observations(identity_id, observed_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_display_name_observations_source
            ON display_name_observations(identity_id, source_message_id, display_name)
            WHERE source_message_id IS NOT NULL;",
    )?;
    Ok(())
}
