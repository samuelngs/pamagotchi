use super::*;

pub(in crate::store::sqlite) fn start_action_run(
    conn: &Connection,
    run: &ActionRunRecord,
) -> anyhow::Result<()> {
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

pub(in crate::store::sqlite) fn get_action_run(
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

pub(in crate::store::sqlite) fn finish_action_run(
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

pub(in crate::store::sqlite) fn read_action_run(
    row: &Row<'_>,
) -> rusqlite::Result<ActionRunRecord> {
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
