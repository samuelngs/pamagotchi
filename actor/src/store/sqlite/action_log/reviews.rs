use super::*;

pub(in crate::store::sqlite) fn mark_review_scheduled(
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

pub(in crate::store::sqlite) fn action_review_scheduled(
    conn: &Connection,
    action_id: &str,
) -> anyhow::Result<bool> {
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

pub(in crate::store::sqlite) fn record_review_output(
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

pub(in crate::store::sqlite) fn review_outputs_for_action(
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

pub(in crate::store::sqlite) fn review_outputs_for_source_action(
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
