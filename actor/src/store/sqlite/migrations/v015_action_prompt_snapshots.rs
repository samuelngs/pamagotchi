use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_prompt_snapshots (
            action_id TEXT NOT NULL,
            turn INTEGER NOT NULL,
            attempt INTEGER NOT NULL,
            prompt_hash TEXT NOT NULL,
            messages_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(action_id, turn, attempt)
        );
        CREATE INDEX IF NOT EXISTS idx_action_prompt_snapshots_action
            ON action_prompt_snapshots(action_id, attempt, turn);",
    )?;
    Ok(())
}
