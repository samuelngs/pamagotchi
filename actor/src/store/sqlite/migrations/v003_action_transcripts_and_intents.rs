use rusqlite::Connection;

pub(super) fn apply(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_runs (
            action_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            task TEXT NOT NULL,
            conversation_id TEXT,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            status TEXT NOT NULL,
            responded INTEGER NOT NULL DEFAULT 0,
            attempts INTEGER NOT NULL DEFAULT 0,
            memories_formed TEXT NOT NULL DEFAULT '[]',
            recalled_memory_ids TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_action_runs_started ON action_runs(started_at);
        CREATE INDEX IF NOT EXISTS idx_action_runs_conversation ON action_runs(conversation_id, started_at);

        CREATE TABLE IF NOT EXISTS action_turns (
            action_id TEXT NOT NULL,
            turn INTEGER NOT NULL,
            attempt INTEGER NOT NULL,
            prompt_hash TEXT NOT NULL,
            model TEXT,
            finish TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            text_len INTEGER NOT NULL DEFAULT 0,
            reasoning_len INTEGER NOT NULL DEFAULT 0,
            tool_call_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(action_id, turn, attempt)
        );
        CREATE INDEX IF NOT EXISTS idx_action_turns_action ON action_turns(action_id, turn);

        CREATE TABLE IF NOT EXISTS action_prompt_snapshots (
            action_id TEXT NOT NULL,
            turn INTEGER NOT NULL,
            attempt INTEGER NOT NULL,
            prompt_hash TEXT NOT NULL,
            messages_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(action_id, turn, attempt)
        );
        CREATE INDEX IF NOT EXISTS idx_action_prompt_snapshots_action
            ON action_prompt_snapshots(action_id, attempt, turn);

        CREATE TABLE IF NOT EXISTS action_tool_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action_id TEXT NOT NULL,
            turn INTEGER NOT NULL,
            call_id TEXT NOT NULL,
            name TEXT NOT NULL,
            args_json TEXT NOT NULL,
            result_json TEXT NOT NULL,
            success INTEGER NOT NULL,
            started_at INTEGER NOT NULL,
            ended_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_action_tool_calls_action ON action_tool_calls(action_id, turn);

        CREATE TABLE IF NOT EXISTS action_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action_id TEXT NOT NULL,
            role TEXT NOT NULL,
            conversation_id TEXT,
            source_gateway_id TEXT,
            source_message_id TEXT,
            sender_external_id TEXT,
            reply_external_id TEXT,
            content TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_action_messages_action ON action_messages(action_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_action_messages_source_unique
            ON action_messages(action_id, role, source_gateway_id, source_message_id)
            WHERE source_gateway_id IS NOT NULL AND source_message_id IS NOT NULL;

        CREATE TABLE IF NOT EXISTS action_outbound_deliveries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action_id TEXT NOT NULL,
            conversation_id TEXT,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            status TEXT NOT NULL,
            error TEXT,
            attempted_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_action_outbound_deliveries_action
            ON action_outbound_deliveries(action_id, attempted_at);

        CREATE TABLE IF NOT EXISTS action_review_watermarks (
            action_id TEXT PRIMARY KEY,
            review_action_id TEXT NOT NULL,
            scheduled_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS review_outputs (
            id TEXT PRIMARY KEY,
            review_action_id TEXT NOT NULL,
            source_action_id TEXT,
            input_json TEXT NOT NULL,
            result_json TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_review_outputs_review_action
            ON review_outputs(review_action_id, applied_at);
        CREATE INDEX IF NOT EXISTS idx_review_outputs_source_action
            ON review_outputs(source_action_id, applied_at);

        CREATE TABLE IF NOT EXISTS intents (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            task TEXT NOT NULL,
            person_id TEXT,
            profile_id TEXT,
            conversation_id TEXT,
            fire_at INTEGER,
            condition TEXT,
            recurrence TEXT,
            priority INTEGER NOT NULL DEFAULT 50,
            dedupe_key TEXT,
            source_action_id TEXT,
            source_memory_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_fired_at INTEGER,
            chosen_person_approved INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_intents_due ON intents(status, fire_at, priority);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_intents_dedupe
            ON intents(dedupe_key)
            WHERE dedupe_key IS NOT NULL;",
    )?;
    Ok(())
}
