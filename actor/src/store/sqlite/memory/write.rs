use super::*;

pub(in crate::store::sqlite) fn store_memory(
    conn: &Connection,
    memory: &Memory,
) -> anyhow::Result<MemoryId> {
    let _slow_query = SlowSqliteQuery::start("store_memory");
    let tx = TxGuard::begin(conn)?;
    let source_json = serde_json::to_string(&memory.source)?;
    let tags_json = serde_json::to_string(&memory.tags)?;
    let evidence_message_ids_json = serde_json::to_string(&memory.evidence_message_ids)?;
    let evidence_json = serde_json::to_string(&memory.evidence)?;
    let existing_id = match memory.dedupe_key.as_deref() {
        Some(dedupe_key) => conn
            .query_row(
                "SELECT id FROM memories WHERE dedupe_key = ?1",
                params![dedupe_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?,
        None => None,
    };
    let was_update = existing_id.is_some();
    let target_id = existing_id.unwrap_or_else(|| memory.id.0.clone());

    if was_update {
        conn.execute(
            "UPDATE memories SET
                kind = ?2,
                memory_type = ?3,
                truth_status = ?4,
                content = ?5,
                source = ?6,
                importance = ?7,
                confidence = ?8,
                sensitivity = ?9,
                sensitivity_category = ?10,
                emotional_valence = ?11,
                accessed_at = ?12,
                tags = ?13,
                evidence_message_ids = ?14,
                evidence_quote = ?15,
                evidence_json = ?16,
                expires_at = ?17,
                stability = ?18,
                supersedes = ?19,
                superseded_by = ?20,
                contradiction_group = ?21,
                privacy_category = ?22,
                visibility_scope = ?23,
                last_confirmed_at = ?24,
                next_review_at = ?25,
                embedding_model = ?26,
                embedding_version = ?27
             WHERE id = ?1",
            params![
                target_id,
                memory.kind.as_str(),
                memory.memory_type.as_str(),
                memory.truth_status.as_str(),
                memory.content,
                source_json,
                memory.importance,
                memory.confidence,
                memory.sensitivity,
                memory.sensitivity_category.as_deref(),
                memory.emotional_valence,
                memory.accessed_at,
                tags_json,
                evidence_message_ids_json,
                memory.evidence_quote.as_deref(),
                evidence_json,
                memory.expires_at,
                memory.stability.as_str(),
                memory.supersedes.as_ref().map(|id| id.0.as_str()),
                memory.superseded_by.as_ref().map(|id| id.0.as_str()),
                memory.contradiction_group.as_deref(),
                memory.privacy_category.as_str(),
                memory.visibility_scope.as_str(),
                memory.last_confirmed_at,
                memory.next_review_at,
                memory.embedding_model.as_deref(),
                memory.embedding_version.as_deref(),
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO memories (
                id, kind, memory_type, truth_status, content, source, importance, confidence,
                sensitivity, sensitivity_category, emotional_valence, created_at, accessed_at,
                access_count, tags, evidence_message_ids, evidence_quote, evidence_json,
                expires_at, stability, supersedes, superseded_by, contradiction_group,
                privacy_category, visibility_scope, last_confirmed_at, next_review_at,
                dedupe_key, embedding_model, embedding_version
             )
             VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30
             )",
            params![
                target_id,
                memory.kind.as_str(),
                memory.memory_type.as_str(),
                memory.truth_status.as_str(),
                memory.content,
                source_json,
                memory.importance,
                memory.confidence,
                memory.sensitivity,
                memory.sensitivity_category.as_deref(),
                memory.emotional_valence,
                memory.created_at,
                memory.accessed_at,
                memory.access_count,
                tags_json,
                evidence_message_ids_json,
                memory.evidence_quote.as_deref(),
                evidence_json,
                memory.expires_at,
                memory.stability.as_str(),
                memory.supersedes.as_ref().map(|id| id.0.as_str()),
                memory.superseded_by.as_ref().map(|id| id.0.as_str()),
                memory.contradiction_group.as_deref(),
                memory.privacy_category.as_str(),
                memory.visibility_scope.as_str(),
                memory.last_confirmed_at,
                memory.next_review_at,
                memory.dedupe_key.as_deref(),
                memory.embedding_model.as_deref(),
                memory.embedding_version.as_deref(),
            ],
        )?;
    }

    conn.execute(
        "DELETE FROM memory_subjects WHERE memory_id = ?1",
        params![target_id],
    )?;
    for subject in &memory.subjects {
        conn.execute(
            "INSERT OR IGNORE INTO memory_subjects (memory_id, subject_type, subject_id, role, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                target_id,
                subject.subject_type.as_str(),
                subject.subject_id,
                subject.role,
                subject.confidence,
            ],
        )?;
    }

    if let Some(ref embedding) = memory.embedding {
        write_memory_embedding_best_effort(conn, &target_id, embedding)?;
    }

    conn.execute(
        "DELETE FROM memories_fts WHERE rowid = (SELECT rowid FROM memories WHERE id = ?1)",
        params![target_id],
    )?;
    conn.execute(
        "INSERT INTO memories_fts (rowid, content) VALUES ((SELECT rowid FROM memories WHERE id = ?1), ?2)",
        params![target_id, memory.content],
    )?;

    conn.execute(
        "INSERT INTO memory_mutations (memory_id, operation, reason, data_json, created_at)
         VALUES (?1, ?2, ?3, ?4, unixepoch())",
        params![
            target_id,
            if was_update {
                "upsert_update"
            } else {
                "create"
            },
            memory.dedupe_key.as_deref(),
            serde_json::json!({
                "input_memory_id": memory.id.0,
                "dedupe_key": memory.dedupe_key,
            })
            .to_string(),
        ],
    )?;

    tx.commit()?;
    Ok(MemoryId(target_id))
}
