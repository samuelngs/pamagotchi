use super::support::SlowSqliteQuery;
use crate::store::{
    ActionMessageRecord, ActionPromptSnapshotRecord, ActionRunRecord, ActionTranscriptRecord,
    ActionTurnRecord, OutboundDeliveryRecord, ReviewOutputAudit, ToolCallRecord,
};
use protocol::{ConversationId, MemoryId};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub(super) fn start_action_run(conn: &Connection, run: &ActionRunRecord) -> anyhow::Result<()> {
    let conversation_id = run.conversation.as_ref().map(|c| c.0.as_str());
    conn.execute(
        "INSERT INTO action_runs (
            action_id, kind, task, conversation_id, started_at, ended_at, status, responded, attempts
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(action_id) DO UPDATE SET
            kind = excluded.kind,
            task = excluded.task,
            conversation_id = excluded.conversation_id,
            started_at = excluded.started_at,
            status = excluded.status",
        params![
            run.action_id.as_str(),
            run.kind.as_str(),
            run.task.as_str(),
            conversation_id,
            run.started_at,
            run.ended_at,
            run.status.as_str(),
            run.responded as i32,
            run.attempts,
        ],
    )?;
    Ok(())
}

pub(super) fn get_action_run(
    conn: &Connection,
    action_id: &str,
) -> anyhow::Result<Option<ActionRunRecord>> {
    let _slow_query = SlowSqliteQuery::start("get_action_run");
    conn.query_row(
        "SELECT action_id, kind, task, conversation_id, started_at, ended_at, status,
                responded, attempts
         FROM action_runs WHERE action_id = ?1",
        params![action_id],
        read_action_run,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn finish_action_run(
    conn: &Connection,
    action_id: &str,
    ended_at: i64,
    status: &str,
    responded: bool,
    attempts: u32,
    memories_formed: &[MemoryId],
    recalled_memory_ids: &[MemoryId],
) -> anyhow::Result<()> {
    let memories_formed_json = serde_json::to_string(memories_formed)?;
    let recalled_memory_ids_json = serde_json::to_string(recalled_memory_ids)?;
    conn.execute(
        "UPDATE action_runs
         SET ended_at = ?2,
             status = ?3,
             responded = ?4,
             attempts = ?5,
             memories_formed = ?6,
             recalled_memory_ids = ?7
         WHERE action_id = ?1",
        params![
            action_id,
            ended_at,
            status,
            responded as i32,
            attempts,
            memories_formed_json,
            recalled_memory_ids_json
        ],
    )?;
    Ok(())
}

fn read_action_run(row: &Row<'_>) -> rusqlite::Result<ActionRunRecord> {
    let conversation: Option<String> = row.get("conversation_id")?;
    Ok(ActionRunRecord {
        action_id: row.get("action_id")?,
        kind: row.get("kind")?,
        task: row.get("task")?,
        conversation: conversation.map(ConversationId),
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        status: row.get("status")?,
        responded: row.get::<_, i32>("responded")? != 0,
        attempts: row.get("attempts")?,
    })
}

pub(super) fn append_action_turn(conn: &Connection, turn: &ActionTurnRecord) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO action_turns (
            action_id, turn, attempt, prompt_hash, model, finish, input_tokens, output_tokens,
            text_len, reasoning_len, tool_call_count, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            turn.action_id.as_str(),
            turn.turn,
            turn.attempt,
            turn.prompt_hash.as_str(),
            turn.model.as_deref(),
            turn.finish.as_deref(),
            turn.input_tokens,
            turn.output_tokens,
            turn.text_len,
            turn.reasoning_len,
            turn.tool_call_count,
            turn.created_at,
        ],
    )?;
    Ok(())
}

pub(super) fn record_prompt_snapshot(
    conn: &Connection,
    snapshot: &ActionPromptSnapshotRecord,
) -> anyhow::Result<()> {
    let messages_json = serde_json::to_string(&snapshot.messages)?;
    conn.execute(
        "INSERT OR REPLACE INTO action_prompt_snapshots (
            action_id, turn, attempt, prompt_hash, messages_json, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot.action_id.as_str(),
            snapshot.turn,
            snapshot.attempt,
            snapshot.prompt_hash.as_str(),
            messages_json,
            snapshot.created_at,
        ],
    )?;
    Ok(())
}

pub(super) fn append_tool_call(conn: &Connection, call: &ToolCallRecord) -> anyhow::Result<()> {
    let args_json = serde_json::to_string(&redact_tool_trace_value(&call.args))?;
    let result_json = serde_json::to_string(&redact_tool_trace_value(&call.result))?;
    conn.execute(
        "INSERT INTO action_tool_calls (
            action_id, turn, call_id, name, args_json, result_json, success, started_at, ended_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            call.action_id.as_str(),
            call.turn,
            call.call_id.as_str(),
            call.name.as_str(),
            args_json,
            result_json,
            call.success as i32,
            call.started_at,
            call.ended_at,
        ],
    )?;
    Ok(())
}

pub(super) fn append_action_message(
    conn: &Connection,
    message: &ActionMessageRecord,
) -> anyhow::Result<()> {
    let conversation_id = message.conversation.as_ref().map(|c| c.0.as_str());
    let source_gateway_id = message.source_gateway_id.as_deref();
    let source_message_id = message.source_message_id.as_deref();
    let sender_external_id = message.sender_external_id.as_deref();
    let reply_external_id = message.reply_external_id.as_deref();
    let content = message.content.as_deref();
    conn.execute(
        "INSERT OR IGNORE INTO action_messages (
            action_id, role, conversation_id, source_gateway_id, source_message_id,
            sender_external_id, reply_external_id, content, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message.action_id.as_str(),
            message.role.as_str(),
            conversation_id,
            source_gateway_id,
            source_message_id,
            sender_external_id,
            reply_external_id,
            content,
            message.created_at,
        ],
    )?;
    Ok(())
}

pub(super) fn append_outbound_delivery(
    conn: &Connection,
    delivery: &OutboundDeliveryRecord,
) -> anyhow::Result<()> {
    let conversation_id = delivery.conversation.as_ref().map(|c| c.0.as_str());
    conn.execute(
        "INSERT INTO action_outbound_deliveries (
            action_id, conversation_id, gateway_id, external_id, status, error, attempted_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            delivery.action_id.as_str(),
            conversation_id,
            delivery.gateway_id.as_str(),
            delivery.external_id.as_str(),
            delivery.status.as_str(),
            delivery.error.as_deref(),
            delivery.attempted_at,
        ],
    )?;
    Ok(())
}

pub(super) fn action_transcript(
    conn: &Connection,
    action_id: &str,
) -> anyhow::Result<ActionTranscriptRecord> {
    let _slow_query = SlowSqliteQuery::start("action_transcript");
    let run_with_artifacts = conn
        .query_row(
            "SELECT action_id, kind, task, conversation_id, started_at, ended_at, status,
                    responded, attempts, memories_formed, recalled_memory_ids
             FROM action_runs WHERE action_id = ?1",
            params![action_id],
            |row| {
                let memories_formed_json: String = row.get("memories_formed")?;
                let recalled_memory_ids_json: String = row.get("recalled_memory_ids")?;
                Ok((
                    read_action_run(row)?,
                    serde_json::from_str::<Vec<MemoryId>>(&memories_formed_json)
                        .unwrap_or_default(),
                    serde_json::from_str::<Vec<MemoryId>>(&recalled_memory_ids_json)
                        .unwrap_or_default(),
                ))
            },
        )
        .optional()?;
    let (run, memories_formed, recalled_memory_ids) = run_with_artifacts
        .map(|(run, memories_formed, recalled_memory_ids)| {
            (Some(run), memories_formed, recalled_memory_ids)
        })
        .unwrap_or((None, vec![], vec![]));

    let mut turns_stmt = conn.prepare(
        "SELECT action_id, turn, attempt, prompt_hash, model, finish, input_tokens,
                output_tokens, text_len, reasoning_len, tool_call_count, created_at
         FROM action_turns
         WHERE action_id = ?1
         ORDER BY attempt ASC, turn ASC",
    )?;
    let turns = turns_stmt
        .query_map(params![action_id], |row| {
            Ok(ActionTurnRecord {
                action_id: row.get("action_id")?,
                turn: row.get("turn")?,
                attempt: row.get("attempt")?,
                prompt_hash: row.get("prompt_hash")?,
                model: row.get("model")?,
                finish: row.get("finish")?,
                input_tokens: row.get("input_tokens")?,
                output_tokens: row.get("output_tokens")?,
                text_len: row.get("text_len")?,
                reasoning_len: row.get("reasoning_len")?,
                tool_call_count: row.get("tool_call_count")?,
                created_at: row.get("created_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();

    let mut snapshots_stmt = conn.prepare(
        "SELECT action_id, turn, attempt, prompt_hash, messages_json, created_at
         FROM action_prompt_snapshots
         WHERE action_id = ?1
         ORDER BY attempt ASC, turn ASC",
    )?;
    let prompt_snapshots = snapshots_stmt
        .query_map(params![action_id], |row| {
            let messages_json: String = row.get("messages_json")?;
            Ok(ActionPromptSnapshotRecord {
                action_id: row.get("action_id")?,
                turn: row.get("turn")?,
                attempt: row.get("attempt")?,
                prompt_hash: row.get("prompt_hash")?,
                messages: serde_json::from_str(&messages_json).unwrap_or(serde_json::Value::Null),
                created_at: row.get("created_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();

    let mut tool_stmt = conn.prepare(
        "SELECT action_id, turn, call_id, name, args_json, result_json, success,
                started_at, ended_at
         FROM action_tool_calls
         WHERE action_id = ?1
         ORDER BY turn ASC, started_at ASC, id ASC",
    )?;
    let tool_calls = tool_stmt
        .query_map(params![action_id], |row| {
            let args_json: String = row.get("args_json")?;
            let result_json: String = row.get("result_json")?;
            Ok(ToolCallRecord {
                action_id: row.get("action_id")?,
                turn: row.get("turn")?,
                call_id: row.get("call_id")?,
                name: row.get("name")?,
                args: serde_json::from_str(&args_json).unwrap_or(serde_json::Value::Null),
                result: serde_json::from_str(&result_json).unwrap_or(serde_json::Value::Null),
                success: row.get::<_, i32>("success")? != 0,
                started_at: row.get("started_at")?,
                ended_at: row.get("ended_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();

    let mut messages_stmt = conn.prepare(
        "SELECT action_id, role, conversation_id, source_gateway_id, source_message_id,
                sender_external_id, reply_external_id, content, created_at
         FROM action_messages
         WHERE action_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let messages = messages_stmt
        .query_map(params![action_id], |row| {
            let conversation: Option<String> = row.get("conversation_id")?;
            Ok(ActionMessageRecord {
                action_id: row.get("action_id")?,
                role: row.get("role")?,
                conversation: conversation.map(ConversationId),
                source_gateway_id: row.get("source_gateway_id")?,
                source_message_id: row.get("source_message_id")?,
                sender_external_id: row.get("sender_external_id")?,
                reply_external_id: row.get("reply_external_id")?,
                content: row.get("content")?,
                created_at: row.get("created_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();

    let mut deliveries_stmt = conn.prepare(
        "SELECT action_id, conversation_id, gateway_id, external_id, status, error, attempted_at
         FROM action_outbound_deliveries
         WHERE action_id = ?1
         ORDER BY attempted_at ASC, id ASC",
    )?;
    let deliveries = deliveries_stmt
        .query_map(params![action_id], |row| {
            let conversation: Option<String> = row.get("conversation_id")?;
            Ok(OutboundDeliveryRecord {
                action_id: row.get("action_id")?,
                conversation: conversation.map(ConversationId),
                gateway_id: row.get("gateway_id")?,
                external_id: row.get("external_id")?,
                status: row.get("status")?,
                error: row.get("error")?,
                attempted_at: row.get("attempted_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();

    Ok(ActionTranscriptRecord {
        run,
        turns,
        prompt_snapshots,
        tool_calls,
        messages,
        deliveries,
        memories_formed,
        recalled_memory_ids,
    })
}

pub(super) fn outbound_deliveries_for_action(
    conn: &Connection,
    action_id: &str,
) -> anyhow::Result<Vec<OutboundDeliveryRecord>> {
    let mut stmt = conn.prepare(
        "SELECT action_id, conversation_id, gateway_id, external_id, status, error, attempted_at
         FROM action_outbound_deliveries
         WHERE action_id = ?1
         ORDER BY attempted_at ASC",
    )?;
    let results = stmt
        .query_map(params![action_id], |row| {
            let conversation: Option<String> = row.get("conversation_id")?;
            Ok(OutboundDeliveryRecord {
                action_id: row.get("action_id")?,
                conversation: conversation.map(ConversationId),
                gateway_id: row.get("gateway_id")?,
                external_id: row.get("external_id")?,
                status: row.get("status")?,
                error: row.get("error")?,
                attempted_at: row.get("attempted_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();
    Ok(results)
}

pub(super) fn mark_review_scheduled(
    conn: &Connection,
    action_id: &str,
    review_action_id: &str,
    scheduled_at: i64,
) -> anyhow::Result<bool> {
    let rows = conn.execute(
        "INSERT OR IGNORE INTO action_review_watermarks (action_id, review_action_id, scheduled_at)
         VALUES (?1, ?2, ?3)",
        params![action_id, review_action_id, scheduled_at],
    )?;
    Ok(rows > 0)
}

pub(super) fn action_review_scheduled(conn: &Connection, action_id: &str) -> anyhow::Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM action_review_watermarks WHERE action_id = ?1",
            params![action_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

pub(super) fn record_review_output(
    conn: &Connection,
    output: &ReviewOutputAudit,
) -> anyhow::Result<()> {
    let input_json = serde_json::to_string(&redact_tool_trace_value(&output.input))?;
    let result_json = serde_json::to_string(&redact_tool_trace_value(&output.result))?;
    conn.execute(
        "INSERT INTO review_outputs (
            id, review_action_id, source_action_id, input_json, result_json, applied_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            output.id.as_str(),
            output.review_action_id.as_str(),
            output.source_action_id.as_deref(),
            input_json,
            result_json,
            output.applied_at,
        ],
    )?;
    Ok(())
}

pub(super) fn review_outputs_for_action(
    conn: &Connection,
    review_action_id: &str,
) -> anyhow::Result<Vec<ReviewOutputAudit>> {
    let mut stmt = conn.prepare(
        "SELECT id, review_action_id, source_action_id, input_json, result_json, applied_at
         FROM review_outputs
         WHERE review_action_id = ?1
         ORDER BY applied_at ASC",
    )?;
    let outputs = stmt
        .query_map(params![review_action_id], review_output_from_row)?
        .filter_map(|row| row.ok())
        .collect();
    Ok(outputs)
}

pub(super) fn review_outputs_for_source_action(
    conn: &Connection,
    source_action_id: &str,
) -> anyhow::Result<Vec<ReviewOutputAudit>> {
    let mut stmt = conn.prepare(
        "SELECT id, review_action_id, source_action_id, input_json, result_json, applied_at
         FROM review_outputs
         WHERE source_action_id = ?1
         ORDER BY applied_at ASC",
    )?;
    let outputs = stmt
        .query_map(params![source_action_id], review_output_from_row)?
        .filter_map(|row| row.ok())
        .collect();
    Ok(outputs)
}

fn review_output_from_row(row: &Row<'_>) -> rusqlite::Result<ReviewOutputAudit> {
    let input_json: String = row.get("input_json")?;
    let result_json: String = row.get("result_json")?;
    Ok(ReviewOutputAudit {
        id: row.get("id")?,
        review_action_id: row.get("review_action_id")?,
        source_action_id: row.get("source_action_id")?,
        input: serde_json::from_str(&input_json).unwrap_or(serde_json::Value::Null),
        result: serde_json::from_str(&result_json).unwrap_or(serde_json::Value::Null),
        applied_at: row.get("applied_at")?,
    })
}

fn redact_tool_trace_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if should_redact_tool_trace_key(key) {
                        serde_json::Value::String("[redacted]".into())
                    } else {
                        redact_tool_trace_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_tool_trace_value).collect())
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                match serde_json::from_str::<serde_json::Value>(text) {
                    Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                        serde_json::Value::String(redact_tool_trace_value(&parsed).to_string())
                    }
                    _ => value.clone(),
                }
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn should_redact_tool_trace_key(key: &str) -> bool {
    matches!(
        key,
        "content"
            | "text"
            | "summary"
            | "comm_style"
            | "response_cadence"
            | "channel_preference"
            | "evidence_quote"
            | "reason"
            | "task"
            | "external_id"
            | "sender_external_id"
            | "reply_external_id"
            | "source_message_id"
            | "media_url"
            | "url"
            | "raw_arguments"
    )
}
