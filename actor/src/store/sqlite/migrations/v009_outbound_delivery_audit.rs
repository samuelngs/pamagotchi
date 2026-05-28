use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_outbound_deliveries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action_id TEXT NOT NULL,
            conversation_id TEXT,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            status TEXT NOT NULL,
            error TEXT,
            attempted_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_action_outbound_deliveries_action
            ON action_outbound_deliveries(action_id, attempted_at);",
    )?;
    Ok(())
}
