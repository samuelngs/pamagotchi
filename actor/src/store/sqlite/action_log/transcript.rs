use super::*;

pub(in crate::store::sqlite) fn action_transcript(
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
