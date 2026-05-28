use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS identity_disclosure_audits (
            id TEXT PRIMARY KEY,
            action_id TEXT NOT NULL,
            requester_person_id TEXT,
            target_person_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            allowed INTEGER NOT NULL DEFAULT 0,
            identity_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_identity_disclosures_target
            ON identity_disclosure_audits(target_person_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_identity_disclosures_action
            ON identity_disclosure_audits(action_id);",
    )?;
    Ok(())
}
