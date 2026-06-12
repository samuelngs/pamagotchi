use super::*;

pub(in crate::store::sqlite) fn update_memory(
    conn: &Connection,
    id: &MemoryId,
    update: &MemoryUpdate,
) -> anyhow::Result<()> {
    let _slow_query = SlowSqliteQuery::start("update_memory");
    let tx = TxGuard::begin(conn)?;
    let mut changed_fields = Vec::new();
    macro_rules! note_changed {
        ($field:ident) => {
            if update.$field.is_some() {
                changed_fields.push(stringify!($field));
            }
        };
    }
    note_changed!(content);
    note_changed!(memory_type);
    note_changed!(truth_status);
    note_changed!(importance);
    note_changed!(confidence);
    note_changed!(sensitivity);
    note_changed!(sensitivity_category);
    note_changed!(emotional_valence);
    note_changed!(tags);
    note_changed!(subjects);
    note_changed!(evidence_message_ids);
    note_changed!(evidence_quote);
    note_changed!(evidence);
    note_changed!(expires_at);
    note_changed!(stability);
    note_changed!(supersedes);
    note_changed!(superseded_by);
    note_changed!(contradiction_group);
    note_changed!(privacy_category);
    note_changed!(visibility_scope);
    note_changed!(last_confirmed_at);
    note_changed!(next_review_at);
    note_changed!(dedupe_key);
    note_changed!(embedding_model);
    note_changed!(embedding_version);
    note_changed!(embedding);

    if let Some(ref content) = update.content {
        conn.execute(
            "UPDATE memories SET content = ?1 WHERE id = ?2",
            params![content, id.0],
        )?;
        conn.execute(
            "UPDATE memories_fts SET content = ?1 WHERE rowid = (SELECT rowid FROM memories WHERE id = ?2)",
            params![content, id.0],
        )?;
    }
    if let Some(ref memory_type) = update.memory_type {
        conn.execute(
            "UPDATE memories SET memory_type = ?1 WHERE id = ?2",
            params![memory_type.as_str(), id.0],
        )?;
    }
    if let Some(ref truth_status) = update.truth_status {
        conn.execute(
            "UPDATE memories SET truth_status = ?1 WHERE id = ?2",
            params![truth_status.as_str(), id.0],
        )?;
    }
    if let Some(importance) = update.importance {
        conn.execute(
            "UPDATE memories SET importance = ?1 WHERE id = ?2",
            params![importance, id.0],
        )?;
    }
    if let Some(confidence) = update.confidence {
        conn.execute(
            "UPDATE memories SET confidence = ?1 WHERE id = ?2",
            params![confidence, id.0],
        )?;
    }
    if let Some(sensitivity) = update.sensitivity {
        conn.execute(
            "UPDATE memories SET sensitivity = ?1 WHERE id = ?2",
            params![sensitivity, id.0],
        )?;
    }
    if let Some(ref category) = update.sensitivity_category {
        conn.execute(
            "UPDATE memories SET sensitivity_category = ?1 WHERE id = ?2",
            params![category, id.0],
        )?;
    }
    if let Some(valence) = update.emotional_valence {
        conn.execute(
            "UPDATE memories SET emotional_valence = ?1 WHERE id = ?2",
            params![valence, id.0],
        )?;
    }
    if let Some(ref tags) = update.tags {
        let tags_json = serde_json::to_string(tags)?;
        conn.execute(
            "UPDATE memories SET tags = ?1 WHERE id = ?2",
            params![tags_json, id.0],
        )?;
    }
    if let Some(ref evidence_message_ids) = update.evidence_message_ids {
        let evidence_message_ids_json = serde_json::to_string(evidence_message_ids)?;
        conn.execute(
            "UPDATE memories SET evidence_message_ids = ?1 WHERE id = ?2",
            params![evidence_message_ids_json, id.0],
        )?;
    }
    if let Some(ref quote) = update.evidence_quote {
        conn.execute(
            "UPDATE memories SET evidence_quote = ?1 WHERE id = ?2",
            params![quote, id.0],
        )?;
    }
    if let Some(ref evidence) = update.evidence {
        let evidence_json = serde_json::to_string(evidence)?;
        conn.execute(
            "UPDATE memories SET evidence_json = ?1 WHERE id = ?2",
            params![evidence_json, id.0],
        )?;
    }
    if let Some(expires_at) = update.expires_at {
        conn.execute(
            "UPDATE memories SET expires_at = ?1 WHERE id = ?2",
            params![expires_at, id.0],
        )?;
    }
    if let Some(ref stability) = update.stability {
        conn.execute(
            "UPDATE memories SET stability = ?1 WHERE id = ?2",
            params![stability.as_str(), id.0],
        )?;
    }
    if let Some(ref supersedes) = update.supersedes {
        conn.execute(
            "UPDATE memories SET supersedes = ?1 WHERE id = ?2",
            params![supersedes.0, id.0],
        )?;
    }
    if let Some(ref superseded_by) = update.superseded_by {
        conn.execute(
            "UPDATE memories SET superseded_by = ?1 WHERE id = ?2",
            params![superseded_by.0, id.0],
        )?;
    }
    if let Some(ref group) = update.contradiction_group {
        conn.execute(
            "UPDATE memories SET contradiction_group = ?1 WHERE id = ?2",
            params![group, id.0],
        )?;
    }
    if let Some(ref privacy_category) = update.privacy_category {
        conn.execute(
            "UPDATE memories SET privacy_category = ?1 WHERE id = ?2",
            params![privacy_category.as_str(), id.0],
        )?;
    }
    if let Some(ref visibility_scope) = update.visibility_scope {
        conn.execute(
            "UPDATE memories SET visibility_scope = ?1 WHERE id = ?2",
            params![visibility_scope.as_str(), id.0],
        )?;
    }
    if let Some(last_confirmed_at) = update.last_confirmed_at {
        conn.execute(
            "UPDATE memories SET last_confirmed_at = ?1 WHERE id = ?2",
            params![last_confirmed_at, id.0],
        )?;
    }
    if let Some(next_review_at) = update.next_review_at {
        conn.execute(
            "UPDATE memories SET next_review_at = ?1 WHERE id = ?2",
            params![next_review_at, id.0],
        )?;
    }
    if let Some(ref dedupe_key) = update.dedupe_key {
        conn.execute(
            "UPDATE memories SET dedupe_key = ?1 WHERE id = ?2",
            params![dedupe_key, id.0],
        )?;
    }
    if let Some(ref embedding_model) = update.embedding_model {
        conn.execute(
            "UPDATE memories SET embedding_model = ?1 WHERE id = ?2",
            params![embedding_model, id.0],
        )?;
    }
    if let Some(ref embedding_version) = update.embedding_version {
        conn.execute(
            "UPDATE memories SET embedding_version = ?1 WHERE id = ?2",
            params![embedding_version, id.0],
        )?;
    }
    if let Some(ref subjects) = update.subjects {
        conn.execute(
            "DELETE FROM memory_subjects WHERE memory_id = ?1",
            params![id.0],
        )?;
        for subject in subjects {
            conn.execute(
                "INSERT OR IGNORE INTO memory_subjects (memory_id, subject_type, subject_id, role, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id.0,
                    subject.subject_type.as_str(),
                    subject.subject_id,
                    subject.role,
                    subject.confidence,
                ],
            )?;
        }
    }
    if let Some(ref embedding) = update.embedding {
        write_memory_embedding_best_effort(conn, &id.0, embedding)?;
    }
    if !changed_fields.is_empty() {
        conn.execute(
            "INSERT INTO memory_mutations (memory_id, operation, reason, data_json, created_at)
             VALUES (?1, 'update', NULL, ?2, unixepoch())",
            params![
                id.0,
                serde_json::json!({ "fields": changed_fields }).to_string(),
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}
