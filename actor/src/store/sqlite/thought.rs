use crate::store::{MemorySubjectType, Thought, ThoughtKind};
use protocol::MemoryId;
use rusqlite::{Connection, params};

pub(super) fn log_thought(conn: &Connection, thought: &Thought) -> anyhow::Result<()> {
    let memories_json = serde_json::to_string(&thought.memories_accessed)?;
    let subjects_json = serde_json::to_string(&thought.subjects)?;
    if let Some(existing) = find_duplicate_thought(conn, thought, &subjects_json)? {
        let merged_memories =
            merge_memory_ids(existing.memories_accessed, &thought.memories_accessed);
        let memories_json = serde_json::to_string(&merged_memories)?;
        conn.execute(
            "UPDATE thoughts
             SET timestamp = ?2,
                 importance = ?3,
                 confidence = ?4,
                 memories_accessed = ?5
             WHERE id = ?1",
            params![
                existing.id,
                existing.timestamp.max(thought.timestamp),
                existing.importance.max(thought.importance.clamp(0.0, 1.0)),
                existing.confidence.max(thought.confidence.clamp(0.0, 1.0)),
                memories_json,
            ],
        )?;
        return Ok(());
    }

    conn.execute(
        "INSERT INTO thoughts (
            timestamp, kind, content, importance, confidence, action_id,
            memories_accessed, subjects
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            thought.timestamp,
            thought.kind.as_str(),
            thought.content,
            thought.importance.clamp(0.0, 1.0),
            thought.confidence.clamp(0.0, 1.0),
            thought.action_id.as_deref(),
            memories_json,
            subjects_json,
        ],
    )?;
    Ok(())
}

struct DuplicateThought {
    id: i64,
    timestamp: i64,
    importance: f32,
    confidence: f32,
    memories_accessed: Vec<MemoryId>,
}

fn find_duplicate_thought(
    conn: &Connection,
    thought: &Thought,
    subjects_json: &str,
) -> anyhow::Result<Option<DuplicateThought>> {
    let canonical_content = canonical_thought_content(&thought.content);
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, content, importance, confidence, memories_accessed
         FROM thoughts
         WHERE kind = ?1
           AND subjects = ?2
           AND ((action_id = ?3) OR (action_id IS NULL AND ?3 IS NULL))
         ORDER BY timestamp DESC, id DESC
         LIMIT 20",
    )?;
    let mut rows = stmt.query(params![
        thought.kind.as_str(),
        subjects_json,
        thought.action_id.as_deref(),
    ])?;
    while let Some(row) = rows.next()? {
        let content: String = row.get("content")?;
        if canonical_thought_content(&content) != canonical_content {
            continue;
        }
        let memories_json: String = row.get("memories_accessed")?;
        return Ok(Some(DuplicateThought {
            id: row.get("id")?,
            timestamp: row.get("timestamp")?,
            importance: row.get("importance")?,
            confidence: row.get("confidence")?,
            memories_accessed: serde_json::from_str(&memories_json).unwrap_or_default(),
        }));
    }
    Ok(None)
}

fn canonical_thought_content(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

fn merge_memory_ids(mut existing: Vec<MemoryId>, incoming: &[MemoryId]) -> Vec<MemoryId> {
    for id in incoming {
        if !existing.iter().any(|existing| existing == id) {
            existing.push(id.clone());
        }
    }
    existing
}

pub(super) fn recent_thoughts(conn: &Connection, limit: usize) -> anyhow::Result<Vec<Thought>> {
    let mut stmt = conn.prepare(
        "SELECT timestamp, kind, content, importance, confidence, action_id,
                memories_accessed, subjects
         FROM (
            SELECT timestamp, kind, content, importance, confidence, action_id,
                   memories_accessed, subjects
            FROM thoughts
            ORDER BY importance DESC, confidence DESC, timestamp DESC, id DESC
            LIMIT ?1
         )
         ORDER BY timestamp ASC",
    )?;
    let thoughts = stmt
        .query_map(params![limit as i64], read_thought)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(thoughts)
}

pub(super) fn recent_thoughts_for_subject(
    conn: &Connection,
    subject_type: MemorySubjectType,
    subject_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<Thought>> {
    let scan_limit = limit.clamp(1, 100).saturating_mul(20).max(100) as i64;
    let mut stmt = conn.prepare(
        "SELECT timestamp, kind, content, importance, confidence, action_id,
                memories_accessed, subjects
         FROM thoughts
         ORDER BY importance DESC, confidence DESC, timestamp DESC, id DESC
         LIMIT ?1",
    )?;
    let mut thoughts = stmt
        .query_map(params![scan_limit], read_thought)?
        .filter_map(|r| r.ok())
        .filter(|thought| {
            thought.subjects.iter().any(|subject| {
                subject.subject_type == subject_type && subject.subject_id == subject_id
            })
        })
        .take(limit)
        .collect::<Vec<_>>();
    thoughts.sort_by_key(|thought| thought.timestamp);
    Ok(thoughts)
}

pub(super) fn thoughts_for_action(
    conn: &Connection,
    action_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<Thought>> {
    let mut stmt = conn.prepare(
        "SELECT timestamp, kind, content, importance, confidence, action_id,
                memories_accessed, subjects
         FROM thoughts
         WHERE action_id = ?1
         ORDER BY timestamp ASC, id ASC
         LIMIT ?2",
    )?;
    let thoughts = stmt
        .query_map(params![action_id, limit as i64], read_thought)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(thoughts)
}

fn read_thought(row: &rusqlite::Row<'_>) -> rusqlite::Result<Thought> {
    let kind_str: String = row.get("kind")?;
    let memories_json: String = row.get("memories_accessed")?;
    let subjects_json: String = row.get("subjects")?;
    Ok(Thought {
        timestamp: row.get("timestamp")?,
        kind: ThoughtKind::parse(&kind_str).unwrap_or(ThoughtKind::Observation),
        content: row.get("content")?,
        importance: row.get("importance")?,
        confidence: row.get("confidence")?,
        action_id: row.get("action_id")?,
        memories_accessed: serde_json::from_str(&memories_json).unwrap_or_default(),
        subjects: serde_json::from_str(&subjects_json).unwrap_or_default(),
    })
}

pub(super) fn prune_stale_thoughts(
    conn: &Connection,
    older_than: i64,
    max_importance: f32,
    max_confidence: f32,
    limit: usize,
) -> anyhow::Result<usize> {
    let rows = conn.execute(
        "DELETE FROM thoughts
         WHERE id IN (
            SELECT id
            FROM thoughts
            WHERE timestamp < ?1
              AND importance <= ?2
              AND confidence <= ?3
            ORDER BY timestamp ASC, id ASC
            LIMIT ?4
         )",
        params![
            older_than,
            max_importance.clamp(0.0, 1.0),
            max_confidence.clamp(0.0, 1.0),
            limit as i64,
        ],
    )?;
    Ok(rows)
}
