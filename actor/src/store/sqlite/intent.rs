use super::rows::read_intent;
use super::support::SlowSqliteQuery;
use crate::store::{IntentRecord, IntentUpdateRecord};
use protocol::{ConversationId, PersonId, ProfileId};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

pub(super) fn create_intent(conn: &Connection, intent: &IntentRecord) -> anyhow::Result<()> {
    let person_id = intent.person.as_ref().map(|id| id.0.as_str());
    let profile_id = intent.profile.as_ref().map(|id| id.0.as_str());
    let conversation_id = intent.conversation.as_ref().map(|id| id.0.as_str());
    let source_memory_id = intent.source_memory.as_ref().map(|id| id.0.as_str());
    conn.execute(
        "INSERT OR IGNORE INTO intents (
            id, kind, status, task, person_id, profile_id, conversation_id, fire_at,
            condition, recurrence, priority, dedupe_key, source_action_id, source_memory_id,
            created_at, updated_at, last_fired_at, owner_approved
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            intent.id.as_str(),
            intent.kind.as_str(),
            intent.status.as_str(),
            intent.task.as_str(),
            person_id,
            profile_id,
            conversation_id,
            intent.fire_at,
            intent.condition.as_deref(),
            intent.recurrence.as_deref(),
            intent.priority,
            intent.dedupe_key.as_deref(),
            intent.source_action.as_deref(),
            source_memory_id,
            intent.created_at,
            intent.updated_at,
            intent.last_fired_at,
            intent.owner_approved,
        ],
    )?;
    Ok(())
}

pub(super) fn get_intent(conn: &Connection, id: &str) -> anyhow::Result<Option<IntentRecord>> {
    conn.query_row(
        "SELECT id, kind, status, task, person_id, profile_id, conversation_id, fire_at,
                condition, recurrence, priority, dedupe_key, source_action_id, source_memory_id,
                created_at, updated_at, last_fired_at, owner_approved
         FROM intents WHERE id = ?1",
        params![id],
        read_intent,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn update_intent(
    conn: &Connection,
    id: &str,
    update: &IntentUpdateRecord,
) -> anyhow::Result<bool> {
    let person_id = update.person.as_ref().map(|id| id.0.as_str());
    let profile_id = update.profile.as_ref().map(|id| id.0.as_str());
    let conversation_id = update.conversation.as_ref().map(|id| id.0.as_str());
    let source_memory_id = update.source_memory.as_ref().map(|id| id.0.as_str());
    let rows = conn.execute(
        "UPDATE intents SET
            kind = COALESCE(?2, kind),
            status = COALESCE(?3, status),
            task = COALESCE(?4, task),
            person_id = COALESCE(?5, person_id),
            profile_id = COALESCE(?6, profile_id),
            conversation_id = COALESCE(?7, conversation_id),
            fire_at = COALESCE(?8, fire_at),
            condition = COALESCE(?9, condition),
            recurrence = COALESCE(?10, recurrence),
            priority = COALESCE(?11, priority),
            dedupe_key = COALESCE(?12, dedupe_key),
            source_memory_id = COALESCE(?13, source_memory_id),
            owner_approved = COALESCE(?14, owner_approved),
            updated_at = ?15
         WHERE id = ?1",
        params![
            id,
            update.kind.as_deref(),
            update.status.as_deref(),
            update.task.as_deref(),
            person_id,
            profile_id,
            conversation_id,
            update.fire_at,
            update.condition.as_deref(),
            update.recurrence.as_deref(),
            update.priority,
            update.dedupe_key.as_deref(),
            source_memory_id,
            update.owner_approved,
            update.updated_at,
        ],
    )?;
    Ok(rows > 0)
}

pub(super) fn cancel_intent(conn: &Connection, id: &str, updated_at: i64) -> anyhow::Result<bool> {
    let rows = conn.execute(
        "UPDATE intents SET status = 'cancelled', updated_at = ?2 WHERE id = ?1",
        params![id, updated_at],
    )?;
    Ok(rows > 0)
}

pub(super) fn complete_intent(
    conn: &Connection,
    id: &str,
    updated_at: i64,
) -> anyhow::Result<bool> {
    let rows = conn.execute(
        "UPDATE intents
         SET status = 'completed', updated_at = ?2
         WHERE id = ?1 AND status IN ('active', 'pending_approval', 'fired')",
        params![id, updated_at],
    )?;
    Ok(rows > 0)
}

pub(super) fn active_intents_for_context(
    conn: &Connection,
    person: Option<&PersonId>,
    profile: Option<&ProfileId>,
    conversation: Option<&ConversationId>,
    now: i64,
    limit: usize,
) -> anyhow::Result<Vec<IntentRecord>> {
    let _slow_query = SlowSqliteQuery::start("active_intents_for_context");
    let person_id = person.map(|id| id.0.as_str());
    let profile_id = profile.map(|id| id.0.as_str());
    let conversation_id = conversation.map(|id| id.0.as_str());
    let mut stmt = conn.prepare(
        "SELECT id, kind, status, task, person_id, profile_id, conversation_id, fire_at,
                condition, recurrence, priority, dedupe_key, source_action_id, source_memory_id,
                created_at, updated_at, last_fired_at, owner_approved
         FROM intents
         WHERE status = 'active'
           AND (
                (?1 IS NOT NULL AND person_id = ?1)
             OR (?2 IS NOT NULL AND profile_id = ?2)
             OR (?3 IS NOT NULL AND conversation_id = ?3)
             OR (person_id IS NULL AND profile_id IS NULL AND conversation_id IS NULL)
           )
         ORDER BY
            CASE WHEN fire_at IS NOT NULL AND fire_at <= ?4 THEN 0 ELSE 1 END ASC,
            priority DESC,
            COALESCE(fire_at, 9223372036854775807) ASC,
            updated_at DESC
         LIMIT ?5",
    )?;
    stmt.query_map(
        params![person_id, profile_id, conversation_id, now, limit as i64],
        read_intent,
    )?
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}

pub(super) fn due_intents(
    conn: &Connection,
    now: i64,
    limit: usize,
) -> anyhow::Result<Vec<IntentRecord>> {
    let _slow_query = SlowSqliteQuery::start("due_intents");
    let mut stmt = conn.prepare(
        "SELECT id, kind, status, task, person_id, profile_id, conversation_id, fire_at,
                condition, recurrence, priority, dedupe_key, source_action_id, source_memory_id,
                created_at, updated_at, last_fired_at, owner_approved
         FROM intents
         WHERE status = 'active' AND fire_at IS NOT NULL AND fire_at <= ?1
         ORDER BY priority DESC, fire_at ASC
         LIMIT ?2",
    )?;
    let candidate_limit = limit.saturating_mul(4).max(limit);
    let candidates = stmt
        .query_map(params![now, candidate_limit as i64], read_intent)?
        .filter_map(|row| row.ok())
        .collect::<Vec<_>>();
    let mut seen_targets = HashSet::new();
    let mut intents = Vec::new();
    for intent in candidates {
        if !seen_targets.insert(intent_coalesce_key(&intent)) {
            continue;
        }
        intents.push(intent);
        if intents.len() >= limit {
            break;
        }
    }
    Ok(intents)
}

pub(super) fn mark_intent_fired(
    conn: &Connection,
    id: &str,
    fired_at: i64,
) -> anyhow::Result<bool> {
    let current = conn
        .query_row(
            "SELECT fire_at, recurrence FROM intents WHERE id = ?1 AND status = 'active'",
            params![id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>("fire_at")?,
                    row.get::<_, Option<String>>("recurrence")?,
                ))
            },
        )
        .optional()?;
    let Some((fire_at, recurrence)) = current else {
        return Ok(false);
    };
    if fire_at.is_some_and(|fire_at| fire_at > fired_at) {
        return Ok(false);
    }

    let rows = if let (Some(fire_at), Some(step_secs)) = (
        fire_at,
        recurrence
            .as_deref()
            .and_then(parse_recurrence_interval_secs),
    ) {
        let next_fire_at = next_recurring_fire_at(fire_at, fired_at, step_secs);
        conn.execute(
            "UPDATE intents
             SET fire_at = ?3, last_fired_at = ?2, updated_at = ?2
             WHERE id = ?1 AND status = 'active' AND fire_at = ?4",
            params![id, fired_at, next_fire_at, fire_at],
        )?
    } else {
        conn.execute(
            "UPDATE intents
             SET status = 'fired', last_fired_at = ?2, updated_at = ?2
             WHERE id = ?1 AND status = 'active'",
            params![id, fired_at],
        )?
    };
    Ok(rows > 0)
}

fn next_recurring_fire_at(previous_fire_at: i64, fired_at: i64, step_secs: i64) -> i64 {
    let step_secs = step_secs.max(1);
    let missed = fired_at.saturating_sub(previous_fire_at) / step_secs;
    previous_fire_at.saturating_add((missed + 1).saturating_mul(step_secs))
}

fn parse_recurrence_interval_secs(recurrence: &str) -> Option<i64> {
    let recurrence = recurrence.trim().to_ascii_lowercase();
    if recurrence.is_empty() {
        return None;
    }
    match recurrence.as_str() {
        "hourly" => return Some(60 * 60),
        "daily" => return Some(24 * 60 * 60),
        "weekly" => return Some(7 * 24 * 60 * 60),
        _ => {}
    }
    if let Some(rest) = recurrence.strip_prefix("every ") {
        return parse_interval_words(rest);
    }
    parse_short_interval(&recurrence).or_else(|| parse_iso_duration(&recurrence))
}

fn parse_interval_words(input: &str) -> Option<i64> {
    let mut parts = input.split_whitespace();
    let count = parts.next()?.parse::<i64>().ok()?;
    let unit = parts.next()?;
    interval_secs(count, unit)
}

fn parse_short_interval(input: &str) -> Option<i64> {
    let split_at = input.find(|c: char| !c.is_ascii_digit())?;
    if split_at == 0 {
        return None;
    }
    let count = input[..split_at].parse::<i64>().ok()?;
    let unit = input[split_at..].trim();
    interval_secs(count, unit)
}

fn parse_iso_duration(input: &str) -> Option<i64> {
    let rest = input.strip_prefix('p')?;
    if let Some(hours) = rest
        .strip_prefix('t')
        .and_then(|rest| rest.strip_suffix('h'))
    {
        return interval_secs(hours.parse::<i64>().ok()?, "hours");
    }
    if let Some(days) = rest.strip_suffix('d') {
        return interval_secs(days.parse::<i64>().ok()?, "days");
    }
    if let Some(weeks) = rest.strip_suffix('w') {
        return interval_secs(weeks.parse::<i64>().ok()?, "weeks");
    }
    None
}

fn interval_secs(count: i64, unit: &str) -> Option<i64> {
    if count <= 0 {
        return None;
    }
    let unit_secs = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 60 * 60,
        "d" | "day" | "days" => 24 * 60 * 60,
        "w" | "week" | "weeks" => 7 * 24 * 60 * 60,
        _ => return None,
    };
    count.checked_mul(unit_secs)
}

fn intent_coalesce_key(intent: &IntentRecord) -> String {
    if let Some(person) = &intent.person {
        format!("person:{}", person.0)
    } else if let Some(conversation) = &intent.conversation {
        format!("conversation:{}", conversation.0)
    } else if let Some(dedupe_key) = &intent.dedupe_key {
        format!("dedupe:{dedupe_key}")
    } else {
        format!("intent:{}", intent.id)
    }
}
