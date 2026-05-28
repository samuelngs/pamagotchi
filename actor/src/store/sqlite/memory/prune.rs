use super::*;

pub(in crate::store::sqlite) fn prune_stale_memories(
    conn: &Connection,
    now: i64,
    older_than: i64,
    max_importance: f32,
    max_confidence: f32,
    max_sensitivity: f32,
    limit: usize,
) -> anyhow::Result<usize> {
    let _slow_query = SlowSqliteQuery::start("prune_stale_memories");
    let tx = TxGuard::begin(conn)?;
    let max_importance = max_importance.clamp(0.0, 1.0);
    let max_confidence = max_confidence.clamp(0.0, 1.0);
    let max_sensitivity = max_sensitivity.clamp(0.0, 1.0);
    let protected_types = [
        "boundary",
        "commitment",
        "identity_claim",
        "correction",
        "procedure",
    ];
    let mut stmt = conn.prepare(
        "SELECT id
         FROM memories
         WHERE (
                (expires_at IS NOT NULL AND expires_at < ?1)
                OR ((truth_status = 'outdated' OR truth_status = 'denied' OR superseded_by IS NOT NULL)
                    AND accessed_at < ?2)
              )
           AND importance <= ?3
           AND confidence <= ?4
           AND sensitivity <= ?5
           AND privacy_category NOT IN ('sensitive', 'secret')
           AND memory_type NOT IN (?6, ?7, ?8, ?9, ?10)
         ORDER BY
            CASE WHEN expires_at IS NOT NULL AND expires_at < ?1 THEN 0 ELSE 1 END,
            accessed_at ASC,
            created_at ASC
         LIMIT ?11",
    )?;
    let ids = stmt
        .query_map(
            params![
                now,
                older_than,
                max_importance,
                max_confidence,
                max_sensitivity,
                protected_types[0],
                protected_types[1],
                protected_types[2],
                protected_types[3],
                protected_types[4],
                limit as i64,
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for id in &ids {
        conn.execute(
            "INSERT INTO memory_mutations (memory_id, operation, reason, data_json, created_at)
             VALUES (?1, 'prune', 'consolidation_prune', ?2, unixepoch())",
            params![
                id,
                serde_json::json!({
                    "now": now,
                    "older_than": older_than,
                    "max_importance": max_importance,
                    "max_confidence": max_confidence,
                    "max_sensitivity": max_sensitivity,
                })
                .to_string(),
            ],
        )?;
        conn.execute(
            "DELETE FROM memories_fts WHERE rowid = (SELECT rowid FROM memories WHERE id = ?1)",
            params![id],
        )?;
        conn.execute("DELETE FROM memories_vec WHERE memory_id = ?1", params![id])?;
        conn.execute(
            "DELETE FROM memory_subjects WHERE memory_id = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
    }

    tx.commit()?;
    Ok(ids.len())
}
