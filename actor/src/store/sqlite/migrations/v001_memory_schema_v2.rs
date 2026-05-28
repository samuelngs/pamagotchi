use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    let columns = common::table_columns(conn, "memories")?;

    for (name, definition) in [
        ("memory_type", "TEXT NOT NULL DEFAULT 'fact'"),
        ("truth_status", "TEXT NOT NULL DEFAULT 'stated'"),
        ("confidence", "REAL NOT NULL DEFAULT 1.0"),
        ("sensitivity_category", "TEXT"),
        ("evidence_message_ids", "TEXT NOT NULL DEFAULT '[]'"),
        ("evidence_quote", "TEXT"),
        ("evidence_json", "TEXT NOT NULL DEFAULT '{}'"),
        ("expires_at", "INTEGER"),
        ("stability", "TEXT NOT NULL DEFAULT 'stable'"),
        ("supersedes", "TEXT"),
        ("superseded_by", "TEXT"),
        ("contradiction_group", "TEXT"),
        ("privacy_category", "TEXT NOT NULL DEFAULT 'personal'"),
        ("visibility_scope", "TEXT NOT NULL DEFAULT 'profile'"),
        ("last_confirmed_at", "INTEGER"),
        ("next_review_at", "INTEGER"),
        ("dedupe_key", "TEXT"),
        ("embedding_model", "TEXT"),
        ("embedding_version", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE memories ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
         CREATE INDEX IF NOT EXISTS idx_memories_truth ON memories(truth_status);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_dedupe
            ON memories(dedupe_key)
            WHERE dedupe_key IS NOT NULL;
         CREATE TABLE IF NOT EXISTS memory_mutations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_id TEXT NOT NULL,
            operation TEXT NOT NULL,
            reason TEXT,
            data_json TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_memory_mutations_memory ON memory_mutations(memory_id, created_at);",
    )?;

    Ok(())
}
