use super::*;

pub(in crate::store::sqlite) fn append_action_turn(
    conn: &Connection,
    turn: &ActionTurnRecord,
) -> anyhow::Result<()> {
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
