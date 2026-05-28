use super::*;

pub(in crate::store::sqlite) fn forget(
    conn: &Connection,
    id: &MemoryId,
    reason: Option<&str>,
) -> anyhow::Result<bool> {
    let tx = TxGuard::begin(conn)?;
    let exists = conn
        .query_row(
            "SELECT 1 FROM memories WHERE id = ?1",
            params![id.0],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        tx.commit()?;
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO memory_mutations (memory_id, operation, reason, data_json, created_at)
         VALUES (?1, 'forget', ?2, '{}', unixepoch())",
        params![id.0, reason],
    )?;
    conn.execute(
        "DELETE FROM memories_fts WHERE rowid = (SELECT rowid FROM memories WHERE id = ?1)",
        params![id.0],
    )?;
    conn.execute(
        "DELETE FROM memories_vec WHERE memory_id = ?1",
        params![id.0],
    )?;
    conn.execute(
        "DELETE FROM memory_subjects WHERE memory_id = ?1",
        params![id.0],
    )?;
    conn.execute("DELETE FROM memories WHERE id = ?1", params![id.0])?;
    tx.commit()?;
    Ok(true)
}
