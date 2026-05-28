use crate::store::{Memory, MemoryType};
use rusqlite::{Connection, types::Value as SqlValue};
use sqlite_vec::sqlite3_vec_init;
use std::sync::Once;
use std::time::{Duration, Instant};
use tracing::{debug_span, span::EnteredSpan, warn};

static INIT_VEC: Once = Once::new();
const SLOW_SQLITE_QUERY_THRESHOLD: Duration = Duration::from_millis(100);

pub(super) struct SlowSqliteQuery {
    label: &'static str,
    start: Instant,
    threshold: Duration,
    _span: EnteredSpan,
}

impl SlowSqliteQuery {
    pub(super) fn start(label: &'static str) -> Self {
        let span = debug_span!(
            target: "actor::store::sqlite",
            "sqlite_store_operation",
            query = label
        );
        Self {
            label,
            start: Instant::now(),
            threshold: SLOW_SQLITE_QUERY_THRESHOLD,
            _span: span.entered(),
        }
    }
}

impl Drop for SlowSqliteQuery {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        if !sqlite_query_is_slow(elapsed, self.threshold) {
            return;
        }

        warn!(
            target: "actor::store::sqlite",
            query = self.label,
            elapsed_ms = elapsed.as_millis() as u64,
            threshold_ms = self.threshold.as_millis() as u64,
            "slow sqlite store operation"
        );
    }
}

pub(super) fn sqlite_query_is_slow(elapsed: Duration, threshold: Duration) -> bool {
    elapsed >= threshold
}

pub(super) fn register_sqlite_vec() {
    INIT_VEC.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
    });
}

pub(super) fn debug_limit(limit: usize) -> usize {
    limit.clamp(1, 200)
}

pub(super) fn message_revision_metadata(
    metadata_json: String,
    update: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> anyhow::Result<String> {
    let mut metadata = serde_json::from_str::<serde_json::Value>(&metadata_json)
        .unwrap_or_else(|_| serde_json::json!({}));
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    let metadata = metadata.as_object_mut().expect("metadata object");
    update(metadata);
    Ok(serde_json::to_string(metadata)?)
}

pub(super) struct RankedMemory {
    pub(super) memory: Memory,
    pub(super) relevance: f32,
}

pub(super) fn subject_filter_clause(subject_filters: &[(&str, &str)]) -> String {
    if subject_filters.is_empty() {
        String::new()
    } else {
        let terms = subject_filters
            .iter()
            .map(|_| "(ms.subject_type = ? AND ms.subject_id = ?)")
            .collect::<Vec<_>>()
            .join(" OR ");
        format!(
            " AND EXISTS (SELECT 1 FROM memory_subjects ms WHERE ms.memory_id = m.id AND ({terms}))"
        )
    }
}

pub(super) fn push_subject_filter_params(
    params: &mut Vec<SqlValue>,
    subject_filters: &[(&str, &str)],
) {
    for (subject_type, subject_id) in subject_filters {
        params.push(SqlValue::Text((*subject_type).to_string()));
        params.push(SqlValue::Text((*subject_id).to_string()));
    }
}

pub(super) fn memory_type_filter_clause(memory_types: &[MemoryType]) -> String {
    if memory_types.is_empty() {
        String::new()
    } else {
        let placeholders = std::iter::repeat_n("?", memory_types.len())
            .collect::<Vec<_>>()
            .join(", ");
        format!(" AND m.memory_type IN ({placeholders})")
    }
}

pub(super) fn push_memory_type_filter_params(
    params: &mut Vec<SqlValue>,
    memory_types: &[MemoryType],
) {
    for memory_type in memory_types {
        params.push(SqlValue::Text(memory_type.as_str().to_string()));
    }
}

pub(super) fn ranked_by_search_order(memories: Vec<Memory>) -> Vec<RankedMemory> {
    let total = memories.len().max(1) as f32;
    memories
        .into_iter()
        .enumerate()
        .map(|(index, memory)| RankedMemory {
            memory,
            relevance: (1.0 - (index as f32 / total)).clamp(0.0, 1.0),
        })
        .collect()
}

pub(super) fn vector_distance_relevance(distance: f32) -> f32 {
    if !distance.is_finite() {
        return 0.0;
    }
    (1.0 / (1.0 + distance.max(0.0))).clamp(0.0, 1.0)
}

pub(super) fn fallback_text_relevance(query: &str, content: &str) -> f32 {
    let query_terms = normalized_terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let content_terms = normalized_terms(content);
    if content_terms.is_empty() {
        return 0.0;
    }

    let query_phrase = query_terms.join(" ");
    let content_phrase = content_terms.join(" ");
    let phrase_score = if content_phrase.contains(&query_phrase) {
        0.25
    } else {
        0.0
    };

    let mut exact = 0.0f32;
    let mut prefix = 0.0f32;
    let mut substring = 0.0f32;
    for term in &query_terms {
        if content_terms.iter().any(|candidate| candidate == term) {
            exact += 1.0;
        } else if content_terms
            .iter()
            .any(|candidate| candidate.starts_with(term))
        {
            prefix += 1.0;
        } else if content_terms
            .iter()
            .any(|candidate| candidate.contains(term))
        {
            substring += 1.0;
        }
    }

    let total = query_terms.len() as f32;
    (phrase_score + exact / total * 0.55 + prefix / total * 0.42 + substring / total * 0.18)
        .clamp(0.0, 1.0)
}

fn normalized_terms(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|term| {
            term.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .collect()
}

pub(super) fn memory_rank_score(memory: &Memory, relevance: f32, now: i64, searched: bool) -> f32 {
    let importance = memory.importance.clamp(0.0, 1.0);
    let confidence = memory.confidence.clamp(0.0, 1.0);
    let age_secs = now.saturating_sub(memory.created_at).max(0) as f32;
    let recency = (1.0 / (1.0 + (age_secs / (86_400.0 * 30.0)))).clamp(0.0, 1.0);
    let access = ((memory.access_count as f32 + 1.0).ln() / 4.0).clamp(0.0, 1.0);

    if searched {
        relevance.clamp(0.0, 1.0) * 0.58
            + importance * 0.24
            + confidence * 0.10
            + recency * 0.06
            + access * 0.02
    } else {
        recency * 0.52 + importance * 0.32 + confidence * 0.10 + access * 0.06
    }
}

pub(super) fn build_fts_query(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().filter(|w| w.len() > 1).collect();
    if words.is_empty() {
        return input.to_string();
    }
    words
        .iter()
        .map(|w| {
            let clean: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{clean}\"")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub(super) fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub(super) fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub(super) struct TxGuard<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> TxGuard<'a> {
    pub(super) fn begin(conn: &'a Connection) -> anyhow::Result<Self> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self { conn, done: false })
    }

    pub(super) fn commit(mut self) -> anyhow::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.done = true;
        Ok(())
    }
}

impl Drop for TxGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}
