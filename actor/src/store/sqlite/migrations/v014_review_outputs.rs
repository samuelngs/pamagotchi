use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS review_outputs (
            id TEXT PRIMARY KEY,
            review_action_id TEXT NOT NULL,
            source_action_id TEXT,
            input_json TEXT NOT NULL,
            result_json TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_review_outputs_review_action
            ON review_outputs(review_action_id, applied_at);
        CREATE INDEX IF NOT EXISTS idx_review_outputs_source_action
            ON review_outputs(source_action_id, applied_at);",
    )?;
    Ok(())
}
