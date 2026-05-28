use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS social_graph (
            person_a TEXT NOT NULL,
            person_b TEXT NOT NULL,
            relation TEXT NOT NULL,
            PRIMARY KEY(person_a, person_b, relation)
        );",
    )?;

    let columns = common::table_columns(conn, "social_graph")?;

    for (name, definition) in [
        ("direction", "TEXT NOT NULL DEFAULT 'bidirectional'"),
        ("confidence", "REAL NOT NULL DEFAULT 0.5"),
        ("status", "TEXT NOT NULL DEFAULT 'stated'"),
        ("evidence_json", "TEXT"),
        ("source_kind", "TEXT NOT NULL DEFAULT 'system'"),
        ("asserted_by_person_id", "TEXT"),
        ("created_at", "INTEGER NOT NULL DEFAULT 0"),
        ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE social_graph ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "UPDATE social_graph
         SET created_at = unixepoch()
         WHERE created_at = 0;
         UPDATE social_graph
         SET updated_at = created_at
         WHERE updated_at = 0;
         UPDATE social_graph
         SET direction = CASE
            WHEN relation IN ('sibling', 'partner', 'coworker', 'friend') THEN 'bidirectional'
            ELSE 'a_to_b'
         END
         WHERE direction IS NULL OR direction = '' OR direction = 'bidirectional';
         CREATE INDEX IF NOT EXISTS idx_social_graph_status ON social_graph(status);
         CREATE INDEX IF NOT EXISTS idx_social_graph_people ON social_graph(person_a, person_b);",
    )?;
    Ok(())
}
