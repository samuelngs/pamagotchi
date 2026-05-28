use super::*;

pub(in crate::store::sqlite) fn append_tool_call(
    conn: &Connection,
    call: &ToolCallRecord,
) -> anyhow::Result<()> {
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
