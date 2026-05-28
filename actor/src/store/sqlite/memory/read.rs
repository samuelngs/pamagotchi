use super::*;

pub(in crate::store::sqlite) fn get_memory(
    conn: &Connection,
    id: &MemoryId,
) -> anyhow::Result<Option<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, memory_type, truth_status, content, source, importance, confidence,
                sensitivity, sensitivity_category, emotional_valence, created_at, accessed_at,
                access_count, tags, evidence_message_ids, evidence_quote, evidence_json,
                expires_at, stability, supersedes, superseded_by, contradiction_group,
                privacy_category, visibility_scope, last_confirmed_at, next_review_at,
                dedupe_key, embedding_model, embedding_version
         FROM memories WHERE id = ?1",
    )?;
    match stmt.query_row(params![id.0], read_memory) {
        Ok(mut memory) => {
            let mut subjects_stmt = conn.prepare(
                "SELECT subject_type, subject_id, role, confidence FROM memory_subjects WHERE memory_id = ?1",
            )?;
            memory.subjects = subjects_stmt
                .query_map(params![id.0], |row| {
                    let subject_type: String = row.get(0)?;
                    Ok(MemorySubject {
                        subject_type: MemorySubjectType::parse(&subject_type)
                            .unwrap_or(MemorySubjectType::Profile),
                        subject_id: row.get(1)?,
                        role: row.get(2)?,
                        confidence: row.get(3)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let Ok(bytes) = conn.query_row(
                "SELECT embedding FROM memories_vec WHERE memory_id = ?1",
                params![id.0],
                |row| row.get::<_, Vec<u8>>(0),
            ) {
                memory.embedding = Some(bytes_to_embedding(&bytes));
            }
            Ok(Some(memory))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(in crate::store::sqlite) fn memories_for_subject(
    conn: &Connection,
    subject_type: MemorySubjectType,
    subject_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<Memory>> {
    let _slow_query = SlowSqliteQuery::start("memories_for_subject");
    let limit = limit.clamp(1, 5_000) as i64;
    let mut stmt = conn.prepare(
        "SELECT m.id, m.kind, m.memory_type, m.truth_status, m.content, m.source,
                m.importance, m.confidence, m.sensitivity, m.sensitivity_category,
                m.emotional_valence, m.created_at, m.accessed_at, m.access_count,
                m.tags, m.evidence_message_ids, m.evidence_quote, m.evidence_json,
                m.expires_at, m.stability, m.supersedes, m.superseded_by,
                m.contradiction_group, m.privacy_category, m.visibility_scope,
                m.last_confirmed_at, m.next_review_at, m.dedupe_key,
                m.embedding_model, m.embedding_version
         FROM memory_subjects ms
         JOIN memories m ON m.id = ms.memory_id
         WHERE ms.subject_type = ?1 AND ms.subject_id = ?2
         ORDER BY m.created_at DESC, m.importance DESC
         LIMIT ?3",
    )?;
    let mut memories = stmt
        .query_map(
            params![subject_type.as_str(), subject_id, limit],
            read_memory,
        )?
        .collect::<Result<Vec<_>, _>>()?;

    let mut subjects_stmt = conn.prepare(
        "SELECT subject_type, subject_id, role, confidence FROM memory_subjects WHERE memory_id = ?1",
    )?;
    for memory in &mut memories {
        memory.subjects = subjects_stmt
            .query_map(params![memory.id.0], |row| {
                let subject_type: String = row.get(0)?;
                Ok(MemorySubject {
                    subject_type: MemorySubjectType::parse(&subject_type)
                        .unwrap_or(MemorySubjectType::Profile),
                    subject_id: row.get(1)?,
                    role: row.get(2)?,
                    confidence: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
    }

    Ok(memories)
}
