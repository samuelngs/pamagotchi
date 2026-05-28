use super::super::support::{
    RankedMemory, build_fts_query, bytes_to_embedding, fallback_text_relevance, memory_rank_score,
    memory_type_filter_clause, push_memory_type_filter_params, push_subject_filter_params,
    ranked_by_search_order, subject_filter_clause, vector_distance_relevance,
};
use super::*;
use crate::store::MemoryType;
use rusqlite::{params_from_iter, types::Value as SqlValue};
use std::collections::HashSet;

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
