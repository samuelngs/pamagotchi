use rusqlite::Connection;

pub(super) fn ensure_migration_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

pub(super) fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    run_migration(conn, 1, "memory_schema_v2", ensure_memory_columns)?;
    run_migration(
        conn,
        2,
        "message_source_and_reply_context",
        ensure_message_columns,
    )?;
    run_migration(
        conn,
        3,
        "action_transcripts_and_intents",
        ensure_action_intent_tables,
    )?;
    run_migration(
        conn,
        4,
        "social_graph_metadata",
        ensure_social_graph_columns,
    )?;
    run_migration(
        conn,
        5,
        "identity_claim_metadata",
        ensure_identity_claim_columns,
    )?;
    run_migration(conn, 6, "thought_metadata", ensure_thought_columns)?;
    run_migration(
        conn,
        7,
        "conversation_summary_metadata",
        ensure_conversation_summary_columns,
    )?;
    run_migration(
        conn,
        8,
        "identity_disclosure_audit",
        ensure_identity_disclosure_audit_table,
    )?;
    run_migration(
        conn,
        9,
        "outbound_delivery_audit",
        ensure_outbound_delivery_table,
    )?;
    run_migration(conn, 10, "event_inbox", ensure_event_inbox_table)?;
    run_migration(
        conn,
        11,
        "action_message_source_dedupe",
        ensure_action_message_source_unique_index,
    )?;
    run_migration(conn, 12, "state_journal", ensure_state_journal_table)?;
    run_migration(
        conn,
        13,
        "display_name_observations",
        ensure_display_name_observations_table,
    )?;
    run_migration(conn, 14, "review_outputs", ensure_review_outputs_table)?;
    run_migration(
        conn,
        15,
        "action_prompt_snapshots",
        ensure_action_prompt_snapshots_table,
    )?;
    run_migration(
        conn,
        16,
        "intent_owner_approval",
        ensure_intent_owner_approval_column,
    )?;
    run_migration(
        conn,
        17,
        "event_inbox_failure_error",
        ensure_event_inbox_failure_error_column,
    )?;
    run_migration(
        conn,
        18,
        "action_run_outcome_memory_artifacts",
        ensure_action_run_outcome_memory_columns,
    )?;
    run_migration(
        conn,
        19,
        "social_graph_direction",
        ensure_social_graph_direction_column,
    )?;
    Ok(())
}

fn run_migration(
    conn: &Connection,
    version: i64,
    name: &str,
    apply: fn(&Connection) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let already_applied: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
        [version],
        |row| row.get(0),
    )?;
    if already_applied {
        return Ok(());
    }

    apply(conn)?;
    conn.execute(
        "INSERT INTO schema_migrations (version, name, applied_at)
         VALUES (?1, ?2, unixepoch())",
        rusqlite::params![version, name],
    )?;
    Ok(())
}

fn ensure_memory_columns(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(memories)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("memory_type", "TEXT NOT NULL DEFAULT 'fact'"),
        ("truth_status", "TEXT NOT NULL DEFAULT 'stated'"),
        ("confidence", "REAL NOT NULL DEFAULT 1.0"),
        ("sensitivity_category", "TEXT"),
        ("evidence_message_ids", "TEXT NOT NULL DEFAULT '[]'"),
        ("evidence_quote", "TEXT"),
        ("evidence_json", "TEXT NOT NULL DEFAULT '{}'"),
        ("expires_at", "INTEGER"),
        ("stability", "TEXT NOT NULL DEFAULT 'stable'"),
        ("supersedes", "TEXT"),
        ("superseded_by", "TEXT"),
        ("contradiction_group", "TEXT"),
        ("privacy_category", "TEXT NOT NULL DEFAULT 'personal'"),
        ("visibility_scope", "TEXT NOT NULL DEFAULT 'profile'"),
        ("last_confirmed_at", "INTEGER"),
        ("next_review_at", "INTEGER"),
        ("dedupe_key", "TEXT"),
        ("embedding_model", "TEXT"),
        ("embedding_version", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE memories ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
         CREATE INDEX IF NOT EXISTS idx_memories_truth ON memories(truth_status);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_dedupe
            ON memories(dedupe_key)
            WHERE dedupe_key IS NOT NULL;
         CREATE TABLE IF NOT EXISTS memory_mutations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_id TEXT NOT NULL,
            operation TEXT NOT NULL,
            reason TEXT,
            data_json TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_memory_mutations_memory ON memory_mutations(memory_id, created_at);",
    )?;

    Ok(())
}

fn ensure_message_columns(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("source_gateway_id", "TEXT"),
        ("source_message_id", "TEXT"),
        ("sender_external_id", "TEXT"),
        ("reply_external_id", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE messages ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_source_unique
            ON messages(conversation_id, source_gateway_id, source_message_id, role)
            WHERE source_gateway_id IS NOT NULL AND source_message_id IS NOT NULL;",
    )?;

    Ok(())
}

fn ensure_action_intent_tables(conn: &Connection) -> anyhow::Result<()> {
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
            owner_approved INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_intents_due ON intents(status, fire_at, priority);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_intents_dedupe
            ON intents(dedupe_key)
            WHERE dedupe_key IS NOT NULL;",
    )?;
    Ok(())
}

fn ensure_review_outputs_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS review_outputs (
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
            ON review_outputs(source_action_id, applied_at);",
    )?;
    Ok(())
}

fn ensure_action_prompt_snapshots_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_prompt_snapshots (
            action_id TEXT NOT NULL,
            turn INTEGER NOT NULL,
            attempt INTEGER NOT NULL,
            prompt_hash TEXT NOT NULL,
            messages_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(action_id, turn, attempt)
        );
        CREATE INDEX IF NOT EXISTS idx_action_prompt_snapshots_action
            ON action_prompt_snapshots(action_id, attempt, turn);",
    )?;
    Ok(())
}

fn ensure_action_run_outcome_memory_columns(conn: &Connection) -> anyhow::Result<()> {
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
        );",
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(action_runs)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("memories_formed", "TEXT NOT NULL DEFAULT '[]'"),
        ("recalled_memory_ids", "TEXT NOT NULL DEFAULT '[]'"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE action_runs ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    Ok(())
}

fn ensure_intent_owner_approval_column(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(intents)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();
    if !columns.contains("owner_approved") {
        conn.execute(
            "ALTER TABLE intents ADD COLUMN owner_approved INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn ensure_event_inbox_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_inbox (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            due_at INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            dedupe_key TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            fired_at INTEGER,
            last_error TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_event_inbox_due
            ON event_inbox(status, due_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_inbox_dedupe
            ON event_inbox(dedupe_key)
            WHERE dedupe_key IS NOT NULL AND status = 'pending';",
    )?;
    Ok(())
}

fn ensure_event_inbox_failure_error_column(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_inbox (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            due_at INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            dedupe_key TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            fired_at INTEGER,
            last_error TEXT
        );",
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(event_inbox)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();
    if !columns.contains("last_error") {
        conn.execute("ALTER TABLE event_inbox ADD COLUMN last_error TEXT", [])?;
    }
    Ok(())
}

fn ensure_action_message_source_unique_index(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "DELETE FROM action_messages
            WHERE id IN (
                SELECT later.id
                FROM action_messages later
                JOIN action_messages earlier
                    ON earlier.action_id = later.action_id
                    AND earlier.role = later.role
                    AND earlier.source_gateway_id = later.source_gateway_id
                    AND earlier.source_message_id = later.source_message_id
                    AND earlier.id < later.id
                WHERE later.source_gateway_id IS NOT NULL
                    AND later.source_message_id IS NOT NULL
            );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_action_messages_source_unique
            ON action_messages(action_id, role, source_gateway_id, source_message_id)
            WHERE source_gateway_id IS NOT NULL AND source_message_id IS NOT NULL;",
    )?;
    Ok(())
}

fn ensure_state_journal_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS state_journal (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_state_journal_id ON state_journal(id);",
    )?;
    Ok(())
}

fn ensure_display_name_observations_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS display_name_observations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            identity_id TEXT NOT NULL,
            profile_id TEXT,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            display_name TEXT NOT NULL,
            source_message_id TEXT,
            observed_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_display_name_observations_identity
            ON display_name_observations(identity_id, observed_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_display_name_observations_source
            ON display_name_observations(identity_id, source_message_id, display_name)
            WHERE source_message_id IS NOT NULL;",
    )?;
    Ok(())
}

fn ensure_social_graph_columns(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS social_graph (
            person_a TEXT NOT NULL,
            person_b TEXT NOT NULL,
            relation TEXT NOT NULL,
            PRIMARY KEY(person_a, person_b, relation)
        );",
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(social_graph)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("direction", "TEXT NOT NULL DEFAULT 'bidirectional'"),
        ("confidence", "REAL NOT NULL DEFAULT 0.5"),
        ("status", "TEXT NOT NULL DEFAULT 'stated'"),
        ("evidence_json", "TEXT"),
        ("source_kind", "TEXT NOT NULL DEFAULT 'system'"),
        ("asserted_by_person_id", "TEXT"),
        ("created_at", "INTEGER NOT NULL DEFAULT 0"),
        ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE social_graph ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "UPDATE social_graph
         SET created_at = unixepoch()
         WHERE created_at = 0;
         UPDATE social_graph
         SET updated_at = created_at
         WHERE updated_at = 0;
         UPDATE social_graph
         SET direction = CASE
            WHEN relation IN ('sibling', 'partner', 'coworker', 'friend') THEN 'bidirectional'
            ELSE 'a_to_b'
         END
         WHERE direction IS NULL OR direction = '' OR direction = 'bidirectional';
         CREATE INDEX IF NOT EXISTS idx_social_graph_status ON social_graph(status);
         CREATE INDEX IF NOT EXISTS idx_social_graph_people ON social_graph(person_a, person_b);",
    )?;
    Ok(())
}

fn ensure_social_graph_direction_column(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(social_graph)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();
    if !columns.contains("direction") {
        conn.execute(
            "ALTER TABLE social_graph ADD COLUMN direction TEXT NOT NULL DEFAULT 'bidirectional'",
            [],
        )?;
    }
    conn.execute_batch(
        "UPDATE social_graph
         SET direction = CASE
            WHEN relation IN ('sibling', 'partner', 'coworker', 'friend') THEN 'bidirectional'
            ELSE 'a_to_b'
         END
         WHERE direction IS NULL OR direction = '' OR direction = 'bidirectional';",
    )?;
    Ok(())
}

fn ensure_identity_claim_columns(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS identity_claims (
            id TEXT PRIMARY KEY,
            claimant_id TEXT NOT NULL,
            claimed_person_id TEXT NOT NULL,
            evidence TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );",
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(identity_claims)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("reason", "TEXT"),
        ("evidence_json", "TEXT NOT NULL DEFAULT '{}'"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE identity_claims ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_claims_status ON identity_claims(status);
         CREATE INDEX IF NOT EXISTS idx_claims_claimant_created ON identity_claims(claimant_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_claims_claimed_created ON identity_claims(claimed_person_id, created_at);",
    )?;
    Ok(())
}

fn ensure_thought_columns(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS thoughts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            memories_accessed TEXT NOT NULL DEFAULT '[]',
            subjects TEXT NOT NULL DEFAULT '[]'
        );",
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(thoughts)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("importance", "REAL NOT NULL DEFAULT 0.5"),
        ("confidence", "REAL NOT NULL DEFAULT 0.5"),
        ("action_id", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE thoughts ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_thoughts_ts ON thoughts(timestamp);
         CREATE INDEX IF NOT EXISTS idx_thoughts_signal ON thoughts(importance, confidence, timestamp);
         CREATE INDEX IF NOT EXISTS idx_thoughts_action ON thoughts(action_id);",
    )?;
    Ok(())
}

fn ensure_conversation_summary_columns(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            gateway_id TEXT,
            identity_id TEXT,
            profile_id TEXT,
            person_id TEXT,
            group_id TEXT,
            summary TEXT,
            summary_covered_message_ids TEXT NOT NULL DEFAULT '[]',
            summary_updated_at INTEGER,
            summary_version INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER NOT NULL,
            last_message_at INTEGER NOT NULL,
            message_count INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    let mut stmt = conn.prepare("PRAGMA table_info(conversations)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .filter_map(|row| row.ok())
        .collect::<std::collections::HashSet<_>>();

    for (name, definition) in [
        ("summary_covered_message_ids", "TEXT NOT NULL DEFAULT '[]'"),
        ("summary_updated_at", "INTEGER"),
        ("summary_version", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE conversations ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    Ok(())
}

fn ensure_identity_disclosure_audit_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS identity_disclosure_audits (
            id TEXT PRIMARY KEY,
            action_id TEXT NOT NULL,
            requester_person_id TEXT,
            target_person_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            allowed INTEGER NOT NULL DEFAULT 0,
            identity_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_identity_disclosures_target
            ON identity_disclosure_audits(target_person_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_identity_disclosures_action
            ON identity_disclosure_audits(action_id);",
    )?;
    Ok(())
}

fn ensure_outbound_delivery_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS action_outbound_deliveries (
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
            ON action_outbound_deliveries(action_id, attempted_at);",
    )?;
    Ok(())
}
