use super::*;

pub(in crate::store::sqlite) fn record_prompt_snapshot(
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
