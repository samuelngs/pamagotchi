use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS identity_claims (
            id TEXT PRIMARY KEY,
            claimant_id TEXT NOT NULL,
            claimed_person_id TEXT NOT NULL,
            evidence TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );",
    )?;

    let columns = common::table_columns(conn, "identity_claims")?;

    for (name, definition) in [
        ("reason", "TEXT"),
        ("evidence_json", "TEXT NOT NULL DEFAULT '{}'"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE identity_claims ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_claims_status ON identity_claims(status);
         CREATE INDEX IF NOT EXISTS idx_claims_claimant_created ON identity_claims(claimant_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_claims_claimed_created ON identity_claims(claimed_person_id, created_at);",
    )?;
    Ok(())
}
