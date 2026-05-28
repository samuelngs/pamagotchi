use super::*;

pub(in crate::store::sqlite) fn memory_mutations_for_memory(
    conn: &Connection,
    id: &MemoryId,
    limit: usize,
) -> anyhow::Result<Vec<MemoryMutationRecord>> {
    let _slow_query = SlowSqliteQuery::start("memory_mutations_for_memory");
    let limit = limit.clamp(1, 100) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, memory_id, operation, reason, data_json, created_at
         FROM memory_mutations
         WHERE memory_id = ?1
         ORDER BY created_at DESC, id DESC
         LIMIT ?2",
    )?;
    stmt.query_map(params![id.0, limit], read_memory_mutation)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn read_memory_mutation(row: &rusqlite::Row) -> rusqlite::Result<MemoryMutationRecord> {
    let data_json: String = row.get("data_json")?;
    let data = serde_json::from_str(&data_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(MemoryMutationRecord {
        id: row.get("id")?,
        memory: MemoryId(row.get("memory_id")?),
        operation: row.get("operation")?,
        reason: row.get("reason")?,
        data,
        created_at: row.get("created_at")?,
    })
}
