use super::common;
use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    let columns = common::table_columns(conn, "social_graph")?;
    if !columns.contains("direction") {
        conn.execute(
            "ALTER TABLE social_graph ADD COLUMN direction TEXT NOT NULL DEFAULT 'bidirectional'",
            [],
        )?;
    }
    conn.execute_batch(
        "UPDATE social_graph
         SET direction = CASE
            WHEN relation IN ('sibling', 'partner', 'coworker', 'friend') THEN 'bidirectional'
            ELSE 'a_to_b'
         END
         WHERE direction IS NULL OR direction = '' OR direction = 'bidirectional';",
    )?;
    Ok(())
}
