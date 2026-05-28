use super::rows::read_memory;
use super::support::{
    RankedMemory, SlowSqliteQuery, TxGuard, build_fts_query, bytes_to_embedding,
    embedding_to_bytes, fallback_text_relevance, memory_rank_score, memory_type_filter_clause,
    push_memory_type_filter_params, push_subject_filter_params, ranked_by_search_order,
    subject_filter_clause, vector_distance_relevance,
};
use crate::store::{
    Memory, MemoryKind, MemoryMutationRecord, MemorySource, MemoryStability, MemorySubject,
    MemorySubjectType, MemoryType, MemoryUpdate, PrivacyCategory, RecallQuery, TruthStatus,
    VisibilityScope,
};
use protocol::MemoryId;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value as SqlValue};
use std::collections::HashSet;

pub(super) fn seed_actor_identity_memories(conn: &Connection) -> anyhow::Result<()> {
    let has_actor_identity = conn.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM memories m
            JOIN memory_subjects ms ON ms.memory_id = m.id
            WHERE ms.subject_type = 'actor'
              AND ms.subject_id = 'self'
              AND m.memory_type IN ('fact', 'identity_claim')
              AND m.truth_status IN ('stated', 'confirmed')
        )",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if has_actor_identity {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    let memory = Memory {
        id: MemoryId("actor-self-identity-pamagotchi".into()),
        kind: MemoryKind::Semantic,
        memory_type: MemoryType::IdentityClaim,
        truth_status: TruthStatus::Confirmed,
        content: "My name is Pamagotchi. I am a transparent digital being with continuity and memory, and I do not pretend to be human.".into(),
        source: MemorySource::External,
        importance: 1.0,
        confidence: 1.0,
        sensitivity: 0.0,
        sensitivity_category: Some("identity".into()),
        emotional_valence: 0.0,
        created_at: now,
        accessed_at: now,
        access_count: 0,
        tags: vec!["identity".into(), "self".into()],
        subjects: vec![MemorySubject::actor(Some("self".into()), 1.0)],
        evidence_message_ids: vec![],
        evidence_quote: None,
        evidence: serde_json::json!({ "source": "system_seed" }),
        expires_at: None,
        stability: MemoryStability::Stable,
        supersedes: None,
        superseded_by: None,
        contradiction_group: None,
        privacy_category: PrivacyCategory::Public,
        visibility_scope: VisibilityScope::Global,
        last_confirmed_at: Some(now),
        next_review_at: None,
        dedupe_key: Some("actor:self:identity".into()),
        embedding_model: None,
        embedding_version: None,
        embedding: None,
    };
    store_memory(conn, &memory)?;
    Ok(())
}

pub(super) fn store_memory(conn: &Connection, memory: &Memory) -> anyhow::Result<MemoryId> {
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
    let target_id = existing_id.unwrap_or_else(|| memory.id.0.clone());
    let was_update = target_id != memory.id.0;

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
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "DELETE FROM memories_vec WHERE memory_id = ?1",
            params![target_id],
        )?;
        conn.execute(
            "INSERT INTO memories_vec (memory_id, embedding) VALUES (?1, ?2)",
            params![target_id, bytes],
        )?;
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

pub(super) fn get_memory(conn: &Connection, id: &MemoryId) -> anyhow::Result<Option<Memory>> {
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

pub(super) fn memories_for_subject(
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

pub(super) fn update_memory(
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
        let bytes = embedding_to_bytes(embedding);
        conn.execute(
            "DELETE FROM memories_vec WHERE memory_id = ?1",
            params![id.0],
        )?;
        conn.execute(
            "INSERT INTO memories_vec (memory_id, embedding) VALUES (?1, ?2)",
            params![id.0, bytes],
        )?;
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

pub(super) fn recall(conn: &Connection, query: &RecallQuery) -> anyhow::Result<Vec<Memory>> {
    let _slow_query = SlowSqliteQuery::start("recall");
    let requested = query.offset.saturating_add(query.limit);
    let fetch_limit =
        requested
            .saturating_mul(8)
            .max(query.limit)
            .max(if query.next_review_before.is_some() {
                200
            } else {
                20
            }) as i64;
    let has_text_query = query
        .text
        .as_ref()
        .map_or(false, |text| !text.trim().is_empty());
    let searched = query.embedding.is_some() || has_text_query;

    let mut subject_filters = Vec::new();
    if let Some(identity) = query.identity.as_ref() {
        subject_filters.push(("identity", identity.0.as_str()));
    }
    if let Some(profile) = query.profile.as_ref() {
        subject_filters.push(("profile", profile.0.as_str()));
    }
    if let Some(person) = query.person.as_ref() {
        subject_filters.push(("person", person.0.as_str()));
    }
    if query.actor {
        subject_filters.push(("actor", "self"));
    }
    let subject_ids: HashSet<String> = if subject_filters.is_empty() {
        HashSet::new()
    } else {
        let mut ids = HashSet::new();
        let mut stmt = conn.prepare(
            "SELECT memory_id FROM memory_subjects WHERE subject_type = ?1 AND subject_id = ?2",
        )?;
        for (subject_type, subject_id) in &subject_filters {
            for id in stmt
                .query_map(params![subject_type, subject_id], |row| {
                    row.get::<_, String>(0)
                })?
                .filter_map(|r| r.ok())
            {
                ids.insert(id);
            }
        }
        ids
    };

    let subject_clause = subject_filter_clause(&subject_filters);
    let memory_type_clause = memory_type_filter_clause(&query.memory_types);
    let mut candidates = if let Some(ref embedding) = query.embedding {
        if !subject_filters.is_empty() || !query.memory_types.is_empty() {
            filtered_vector_recall(conn, embedding, &subject_filters, &query.memory_types)?
        } else {
            let bytes = embedding_to_bytes(embedding);
            let sql = format!(
            "SELECT m.id, m.kind, m.memory_type, m.truth_status, m.content, m.source,
                    m.importance, m.confidence, m.sensitivity, m.sensitivity_category,
                    m.emotional_valence, m.created_at, m.accessed_at, m.access_count,
                    m.tags, m.evidence_message_ids, m.evidence_quote, m.evidence_json,
                    m.expires_at, m.stability, m.supersedes, m.superseded_by,
                    m.contradiction_group, m.privacy_category, m.visibility_scope,
                    m.last_confirmed_at, m.next_review_at, m.dedupe_key,
                    m.embedding_model, m.embedding_version, v.distance
		             FROM (SELECT memory_id, distance FROM memories_vec WHERE embedding MATCH ?1 AND k = ?2) v
		             JOIN memories m ON m.id = v.memory_id
		             ORDER BY v.distance, m.importance DESC, m.created_at DESC
		             LIMIT ?"
        );
            let mut sql_params = vec![SqlValue::Blob(bytes), SqlValue::Integer(fetch_limit)];
            sql_params.push(SqlValue::Integer(fetch_limit));
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(params_from_iter(sql_params), |row| {
                let memory = read_memory(row)?;
                let distance = row.get::<_, f64>("distance")? as f32;
                Ok(RankedMemory {
                    memory,
                    relevance: vector_distance_relevance(distance),
                })
            })?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
        }
    } else if let Some(text) = query.text.as_ref().filter(|text| !text.trim().is_empty()) {
        let fts_query = build_fts_query(text);
        let sql = format!(
            "SELECT m.id, m.kind, m.memory_type, m.truth_status, m.content, m.source,
                    m.importance, m.confidence, m.sensitivity, m.sensitivity_category,
                    m.emotional_valence, m.created_at, m.accessed_at, m.access_count,
                    m.tags, m.evidence_message_ids, m.evidence_quote, m.evidence_json,
                    m.expires_at, m.stability, m.supersedes, m.superseded_by,
                    m.contradiction_group, m.privacy_category, m.visibility_scope,
                    m.last_confirmed_at, m.next_review_at, m.dedupe_key,
	                    m.embedding_model, m.embedding_version
	             FROM memories_fts f
	             JOIN memories m ON m.rowid = f.rowid
	             WHERE memories_fts MATCH ?{subject_clause}{memory_type_clause}
	             ORDER BY bm25(memories_fts) ASC, m.importance DESC, m.created_at DESC
	             LIMIT ?"
        );
        let mut sql_params = vec![SqlValue::Text(fts_query)];
        push_subject_filter_params(&mut sql_params, &subject_filters);
        push_memory_type_filter_params(&mut sql_params, &query.memory_types);
        sql_params.push(SqlValue::Integer(fetch_limit));
        let mut stmt = conn.prepare(&sql)?;
        let results: Vec<_> = stmt
            .query_map(params_from_iter(sql_params), read_memory)?
            .filter_map(|r| r.ok())
            .collect();
        if results.is_empty() {
            let fallback_fetch_limit = fetch_limit.max(200);
            let escaped = text
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            let fallback_sql = format!(
                "SELECT m.id, m.kind, m.memory_type, m.truth_status, m.content, m.source, m.importance,
                        confidence, sensitivity, sensitivity_category, emotional_valence,
                        created_at, accessed_at, access_count, tags, evidence_message_ids,
                        evidence_quote, evidence_json, expires_at, stability, supersedes,
                        superseded_by, contradiction_group, privacy_category, visibility_scope,
                        last_confirmed_at, next_review_at, dedupe_key, embedding_model,
                        embedding_version
	                 FROM memories m WHERE content LIKE ? ESCAPE '\\'{subject_clause}{memory_type_clause}
	                 ORDER BY importance DESC, created_at DESC
	                 LIMIT ?"
            );
            let mut fallback_params = vec![SqlValue::Text(pattern)];
            push_subject_filter_params(&mut fallback_params, &subject_filters);
            push_memory_type_filter_params(&mut fallback_params, &query.memory_types);
            fallback_params.push(SqlValue::Integer(fallback_fetch_limit));
            let mut fallback = conn.prepare(&fallback_sql)?;
            let fallback_results = fallback
                .query_map(params_from_iter(fallback_params), read_memory)?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            fallback_results
                .into_iter()
                .map(|memory| {
                    let relevance = fallback_text_relevance(text, &memory.content);
                    RankedMemory { memory, relevance }
                })
                .collect()
        } else {
            ranked_by_search_order(results)
        }
    } else {
        let sql = format!(
            "SELECT m.id, m.kind, m.memory_type, m.truth_status, m.content, m.source, m.importance,
                    confidence, sensitivity, sensitivity_category, emotional_valence,
                    created_at, accessed_at, access_count, tags, evidence_message_ids,
                    evidence_quote, evidence_json, expires_at, stability, supersedes,
                    superseded_by, contradiction_group, privacy_category, visibility_scope,
	                    last_confirmed_at, next_review_at, dedupe_key, embedding_model,
	                    embedding_version
	             FROM memories m
	             WHERE 1=1{subject_clause}{memory_type_clause}
	             ORDER BY created_at DESC, importance DESC
	             LIMIT ?"
        );
        let mut sql_params = Vec::new();
        push_subject_filter_params(&mut sql_params, &subject_filters);
        push_memory_type_filter_params(&mut sql_params, &query.memory_types);
        sql_params.push(SqlValue::Integer(fetch_limit));
        let mut stmt = conn.prepare(&sql)?;
        stmt.query_map(params_from_iter(sql_params), read_memory)?
            .filter_map(|r| r.ok())
            .map(|memory| RankedMemory {
                memory,
                relevance: 0.0,
            })
            .collect::<Vec<_>>()
    };

    candidates.retain(|candidate| {
        let m = &candidate.memory;
        if let Some(ref kind) = query.kind {
            if m.kind.as_str() != kind.as_str() {
                return false;
            }
        }
        if !query.memory_types.is_empty()
            && !query
                .memory_types
                .iter()
                .any(|memory_type| m.memory_type == *memory_type)
        {
            return false;
        }
        if let Some(min_imp) = query.min_importance {
            if m.importance < min_imp {
                return false;
            }
        }
        if let Some(ref range) = query.time_range {
            if let Some(start) = range.start {
                if m.created_at < start {
                    return false;
                }
            }
            if let Some(end) = range.end {
                if m.created_at > end {
                    return false;
                }
            }
        }
        if let Some(max_sens) = query.max_sensitivity {
            if m.sensitivity > max_sens {
                return false;
            }
        }
        if let Some(next_review_before) = query.next_review_before {
            if m.next_review_at.is_none_or(|due| due > next_review_before) {
                return false;
            }
        }
        if !query.include_sensitive
            && matches!(
                m.privacy_category,
                PrivacyCategory::Sensitive | PrivacyCategory::Secret
            )
        {
            return false;
        }
        if !query.include_superseded
            && (m.superseded_by.is_some() || matches!(m.truth_status, TruthStatus::Outdated))
        {
            return false;
        }
        if !subject_filters.is_empty() && !subject_ids.contains(&m.id.0) {
            return false;
        }
        true
    });

    let now = chrono::Utc::now().timestamp();
    candidates.sort_by(|a, b| {
        memory_rank_score(&b.memory, b.relevance, now, searched)
            .total_cmp(&memory_rank_score(&a.memory, a.relevance, now, searched))
            .then_with(|| b.memory.created_at.cmp(&a.memory.created_at))
            .then_with(|| b.memory.importance.total_cmp(&a.memory.importance))
    });

    if query.offset > 0 {
        candidates.drain(..query.offset.min(candidates.len()));
    }
    candidates.truncate(query.limit);

    let mut subjects_stmt = conn.prepare(
        "SELECT subject_type, subject_id, role, confidence FROM memory_subjects WHERE memory_id = ?1",
    )?;
    let mut access_stmt = conn.prepare(
        "UPDATE memories SET accessed_at = unixepoch(), access_count = access_count + 1 WHERE id = ?1",
    )?;
    for candidate in &mut candidates {
        candidate.memory.subjects = subjects_stmt
            .query_map(params![candidate.memory.id.0], |row| {
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
        let _ = access_stmt.execute(params![candidate.memory.id.0]);
    }

    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.memory)
        .collect())
}

fn filtered_vector_recall(
    conn: &Connection,
    query_embedding: &[f32],
    subject_filters: &[(&str, &str)],
    memory_types: &[MemoryType],
) -> anyhow::Result<Vec<RankedMemory>> {
    let subject_clause = subject_filter_clause(subject_filters);
    let memory_type_clause = memory_type_filter_clause(memory_types);
    let sql = format!(
        "SELECT m.id, m.kind, m.memory_type, m.truth_status, m.content, m.source,
                m.importance, m.confidence, m.sensitivity, m.sensitivity_category,
                m.emotional_valence, m.created_at, m.accessed_at, m.access_count,
                m.tags, m.evidence_message_ids, m.evidence_quote, m.evidence_json,
                m.expires_at, m.stability, m.supersedes, m.superseded_by,
                m.contradiction_group, m.privacy_category, m.visibility_scope,
                m.last_confirmed_at, m.next_review_at, m.dedupe_key,
                m.embedding_model, m.embedding_version, v.embedding AS embedding_bytes
         FROM memories m
         JOIN memories_vec v ON v.memory_id = m.id
         WHERE 1=1{subject_clause}{memory_type_clause}"
    );
    let mut sql_params = Vec::new();
    push_subject_filter_params(&mut sql_params, subject_filters);
    push_memory_type_filter_params(&mut sql_params, memory_types);
    let mut stmt = conn.prepare(&sql)?;
    let mut candidates = stmt
        .query_map(params_from_iter(sql_params), |row| {
            let memory = read_memory(row)?;
            let embedding_bytes = row.get::<_, Vec<u8>>("embedding_bytes")?;
            let distance =
                embedding_l2_distance(query_embedding, &bytes_to_embedding(&embedding_bytes));
            Ok(RankedMemory {
                memory,
                relevance: vector_distance_relevance(distance),
            })
        })?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();

    candidates.sort_by(|a, b| {
        b.relevance
            .total_cmp(&a.relevance)
            .then_with(|| b.memory.importance.total_cmp(&a.memory.importance))
            .then_with(|| b.memory.created_at.cmp(&a.memory.created_at))
    });
    Ok(candidates)
}

fn embedding_l2_distance(query: &[f32], candidate: &[f32]) -> f32 {
    if query.is_empty() || candidate.is_empty() || query.len() != candidate.len() {
        return f32::INFINITY;
    }
    query
        .iter()
        .zip(candidate)
        .map(|(a, b)| {
            let delta = a - b;
            delta * delta
        })
        .sum::<f32>()
        .sqrt()
}

pub(super) fn forget(
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

pub(super) fn memory_mutations_for_memory(
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

pub(super) fn prune_stale_memories(
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
