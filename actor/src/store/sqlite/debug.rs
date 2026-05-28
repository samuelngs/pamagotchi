use super::rows::{read_intent, read_memory, read_person_profile_link, read_profile_identity_link};
use super::support::{SlowSqliteQuery, debug_limit};
use crate::identity::{PersonProfileLink, ProfileIdentityLink};
use crate::store::{
    ActionRunRecord, EventInboxDebugRecord, IntentRecord, Memory, MemoryMutationRecord,
    MemorySubject, MemorySubjectDebugRecord, MemorySubjectType, ReviewJobRecord, ReviewOutputAudit,
};
use protocol::{ConversationId, MemoryId};
use rusqlite::{Connection, params};

pub(super) fn recent_memories(conn: &Connection, limit: usize) -> anyhow::Result<Vec<Memory>> {
    let _slow_query = SlowSqliteQuery::start("debug_recent_memories");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT id, kind, memory_type, truth_status, content, source, importance,
                confidence, sensitivity, sensitivity_category, emotional_valence,
                created_at, accessed_at, access_count, tags, evidence_message_ids,
                evidence_quote, evidence_json, expires_at, stability, supersedes,
                superseded_by, contradiction_group, privacy_category, visibility_scope,
                last_confirmed_at, next_review_at, dedupe_key, embedding_model,
                embedding_version
         FROM memories
         ORDER BY created_at DESC, id DESC
         LIMIT ?1",
    )?;
    let mut memories = stmt
        .query_map(params![limit as i64], read_memory)?
        .collect::<Result<Vec<_>, _>>()?;

    let mut subjects_stmt = conn.prepare(
        "SELECT subject_type, subject_id, role, confidence
         FROM memory_subjects
         WHERE memory_id = ?1
         ORDER BY subject_type, subject_id",
    )?;
    for memory in &mut memories {
        memory.subjects = subjects_stmt
            .query_map(params![memory.id.0.as_str()], |row| {
                let subject_type: String = row.get(0)?;
                Ok(MemorySubject {
                    subject_type: MemorySubjectType::parse(&subject_type)
                        .unwrap_or(MemorySubjectType::Profile),
                    subject_id: row.get(1)?,
                    role: row.get(2)?,
                    confidence: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(memories)
}

pub(super) fn memory_subjects(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<MemorySubjectDebugRecord>> {
    let _slow_query = SlowSqliteQuery::start("debug_memory_subjects");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT s.subject_type, s.subject_id, COUNT(*) AS memory_count, MAX(m.created_at) AS latest_memory_at
         FROM memory_subjects s
         JOIN memories m ON m.id = s.memory_id
         GROUP BY s.subject_type, s.subject_id
         ORDER BY latest_memory_at DESC, memory_count DESC, s.subject_type, s.subject_id
         LIMIT ?1",
    )?;
    let mut records = stmt
        .query_map(params![limit as i64], |row| {
            let subject_type: String = row.get("subject_type")?;
            Ok(MemorySubjectDebugRecord {
                subject_type: MemorySubjectType::parse(&subject_type)
                    .unwrap_or(MemorySubjectType::Profile),
                subject_id: row.get("subject_id")?,
                memory_count: row.get::<_, i64>("memory_count")? as u32,
                latest_memory_at: row.get("latest_memory_at")?,
                latest_memory_ids: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut latest_stmt = conn.prepare(
        "SELECT m.id
         FROM memory_subjects s
         JOIN memories m ON m.id = s.memory_id
         WHERE s.subject_type = ?1 AND s.subject_id = ?2
         ORDER BY m.created_at DESC, m.id DESC
         LIMIT 5",
    )?;
    for record in &mut records {
        record.latest_memory_ids = latest_stmt
            .query_map(
                params![record.subject_type.as_str(), record.subject_id.as_str()],
                |row| Ok(MemoryId(row.get(0)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
    }

    Ok(records)
}

pub(super) fn profile_identity_links(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<ProfileIdentityLink>> {
    let _slow_query = SlowSqliteQuery::start("debug_profile_identity_links");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT profile_id, identity_id, status, confidence, evidence_json, created_at, removed_at
         FROM profile_identities
         ORDER BY created_at DESC, profile_id, identity_id
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], read_profile_identity_link)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn person_profile_links(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<PersonProfileLink>> {
    let _slow_query = SlowSqliteQuery::start("debug_person_profile_links");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT person_id, profile_id, status, confidence, evidence_json, created_at, updated_at, detached_at
         FROM person_profiles
         ORDER BY updated_at DESC, created_at DESC, person_id, profile_id
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], read_person_profile_link)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn active_intents(conn: &Connection, limit: usize) -> anyhow::Result<Vec<IntentRecord>> {
    let _slow_query = SlowSqliteQuery::start("debug_active_intents");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT id, kind, status, task, person_id, profile_id, conversation_id, fire_at,
                condition, recurrence, priority, dedupe_key, source_action_id, source_memory_id,
                created_at, updated_at, last_fired_at, owner_approved
         FROM intents
         WHERE status IN ('active', 'pending_approval')
         ORDER BY
            CASE WHEN fire_at IS NULL THEN 1 ELSE 0 END ASC,
            COALESCE(fire_at, 9223372036854775807) ASC,
            priority DESC,
            updated_at DESC
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], read_intent)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn recent_review_outputs(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<ReviewOutputAudit>> {
    let _slow_query = SlowSqliteQuery::start("debug_recent_review_outputs");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT id, review_action_id, source_action_id, input_json, result_json, applied_at
         FROM review_outputs
         ORDER BY applied_at DESC, id DESC
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], |row| {
        let input_json: String = row.get("input_json")?;
        let result_json: String = row.get("result_json")?;
        let input = serde_json::from_str(&input_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let result = serde_json::from_str(&result_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(ReviewOutputAudit {
            id: row.get("id")?,
            review_action_id: row.get("review_action_id")?,
            source_action_id: row.get("source_action_id")?,
            input,
            result,
            applied_at: row.get("applied_at")?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}

pub(super) fn recent_review_jobs(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<ReviewJobRecord>> {
    let _slow_query = SlowSqliteQuery::start("debug_recent_review_jobs");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT
            w.action_id AS source_action_id,
            w.review_action_id,
            w.scheduled_at,
            source.kind AS source_kind,
            source.status AS source_status,
            source.started_at AS source_started_at,
            source.ended_at AS source_ended_at,
            review.status AS review_status,
            review.started_at AS review_started_at,
            review.ended_at AS review_ended_at,
            COALESCE(outputs.output_count, 0) AS output_count,
            outputs.last_applied_at
         FROM action_review_watermarks w
         LEFT JOIN action_runs source ON source.action_id = w.action_id
         LEFT JOIN action_runs review ON review.action_id = w.review_action_id
         LEFT JOIN (
            SELECT review_action_id, COUNT(*) AS output_count, MAX(applied_at) AS last_applied_at
            FROM review_outputs
            GROUP BY review_action_id
         ) outputs ON outputs.review_action_id = w.review_action_id
         ORDER BY w.scheduled_at DESC, w.action_id DESC
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], |row| {
        Ok(ReviewJobRecord {
            source_action_id: row.get("source_action_id")?,
            review_action_id: row.get("review_action_id")?,
            scheduled_at: row.get("scheduled_at")?,
            source_kind: row.get("source_kind")?,
            source_status: row.get("source_status")?,
            source_started_at: row.get("source_started_at")?,
            source_ended_at: row.get("source_ended_at")?,
            review_status: row.get("review_status")?,
            review_started_at: row.get("review_started_at")?,
            review_ended_at: row.get("review_ended_at")?,
            output_count: row.get::<_, i64>("output_count")? as u32,
            last_applied_at: row.get("last_applied_at")?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}

pub(super) fn recent_action_runs(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<ActionRunRecord>> {
    let _slow_query = SlowSqliteQuery::start("debug_recent_action_runs");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT action_id, kind, task, conversation_id, started_at, ended_at,
                status, responded, attempts
         FROM action_runs
         ORDER BY started_at DESC, action_id DESC
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], |row| {
        let conversation_id: Option<String> = row.get("conversation_id")?;
        Ok(ActionRunRecord {
            action_id: row.get("action_id")?,
            kind: row.get("kind")?,
            task: row.get("task")?,
            conversation: conversation_id.map(ConversationId),
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
            status: row.get("status")?,
            responded: row.get::<_, i64>("responded")? != 0,
            attempts: row.get("attempts")?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}

pub(super) fn recent_memory_mutations(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<MemoryMutationRecord>> {
    let _slow_query = SlowSqliteQuery::start("debug_recent_memory_mutations");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT id, memory_id, operation, reason, data_json, created_at
         FROM memory_mutations
         ORDER BY created_at DESC, id DESC
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], |row| {
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
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}

pub(super) fn recent_failed_events(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<EventInboxDebugRecord>> {
    let _slow_query = SlowSqliteQuery::start("debug_recent_failed_events");
    let limit = debug_limit(limit);
    let mut stmt = conn.prepare(
        "SELECT id, kind, status, due_at, attempts, dedupe_key,
                created_at, updated_at, fired_at, last_error
         FROM event_inbox
         WHERE status = 'failed'
         ORDER BY updated_at DESC, created_at DESC, id DESC
         LIMIT ?1",
    )?;
    stmt.query_map(params![limit as i64], |row| {
        Ok(EventInboxDebugRecord {
            id: row.get("id")?,
            kind: row.get("kind")?,
            status: row.get("status")?,
            due_at: row.get("due_at")?,
            attempts: row.get("attempts")?,
            dedupe_key: row.get("dedupe_key")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            fired_at: row.get("fired_at")?,
            last_error: row.get("last_error")?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}
