use super::migrations::{ensure_migration_table, record_clean_schema};
use rusqlite::Connection;

pub(super) fn init_schema(conn: &Connection, embedding_dims: usize) -> anyhow::Result<()> {
    ensure_migration_table(conn)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            memory_type TEXT NOT NULL DEFAULT 'fact',
            truth_status TEXT NOT NULL DEFAULT 'stated',
            content TEXT NOT NULL,
            source TEXT NOT NULL,
            importance REAL NOT NULL DEFAULT 0.5,
            confidence REAL NOT NULL DEFAULT 1.0,
            sensitivity REAL NOT NULL DEFAULT 0.0,
            sensitivity_category TEXT,
            emotional_valence REAL NOT NULL DEFAULT 0.0,
            created_at INTEGER NOT NULL,
            accessed_at INTEGER NOT NULL,
            access_count INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]',
            evidence_message_ids TEXT NOT NULL DEFAULT '[]',
            evidence_quote TEXT,
            evidence_json TEXT NOT NULL DEFAULT '{}',
            expires_at INTEGER,
            stability TEXT NOT NULL DEFAULT 'stable',
            supersedes TEXT,
            superseded_by TEXT,
            contradiction_group TEXT,
            privacy_category TEXT NOT NULL DEFAULT 'personal',
            visibility_scope TEXT NOT NULL DEFAULT 'profile',
            last_confirmed_at INTEGER,
            next_review_at INTEGER,
            dedupe_key TEXT,
            embedding_model TEXT,
            embedding_version TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_memories_kind ON memories(kind);
        CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance);
        CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);

        CREATE TABLE IF NOT EXISTS conversations (
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
        );

        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            identity_id TEXT,
            profile_id TEXT,
            person_id TEXT,
            source_gateway_id TEXT,
            source_message_id TEXT,
            sender_external_id TEXT,
            reply_external_id TEXT,
            metadata TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, timestamp);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_source_unique
            ON messages(conversation_id, source_gateway_id, source_message_id, role)
            WHERE source_gateway_id IS NOT NULL AND source_message_id IS NOT NULL;

        CREATE TABLE IF NOT EXISTS memory_subjects (
            memory_id TEXT NOT NULL,
            subject_type TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            role TEXT,
            confidence REAL NOT NULL,
            PRIMARY KEY(memory_id, subject_type, subject_id, role)
        );
        CREATE INDEX IF NOT EXISTS idx_memory_subjects_subject ON memory_subjects(subject_type, subject_id);

        CREATE TABLE IF NOT EXISTS memory_mutations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_id TEXT NOT NULL,
            operation TEXT NOT NULL,
            reason TEXT,
            data_json TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_mutations_memory ON memory_mutations(memory_id, created_at);

        CREATE TABLE IF NOT EXISTS thoughts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            importance REAL NOT NULL DEFAULT 0.5,
            confidence REAL NOT NULL DEFAULT 0.5,
            action_id TEXT,
            memories_accessed TEXT NOT NULL DEFAULT '[]',
            subjects TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_thoughts_ts ON thoughts(timestamp);

        CREATE TABLE IF NOT EXISTS action_runs (
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
            chosen_human_approved INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_intents_due ON intents(status, fire_at, priority);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_intents_dedupe
            ON intents(dedupe_key)
            WHERE dedupe_key IS NOT NULL;

        CREATE TABLE IF NOT EXISTS event_inbox (
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
            WHERE dedupe_key IS NOT NULL AND status = 'pending';

        CREATE TABLE IF NOT EXISTS snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            saved_at INTEGER NOT NULL,
            data TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_snapshots_saved ON snapshots(saved_at);

        CREATE TABLE IF NOT EXISTS state_journal (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_state_journal_id ON state_journal(id);

        CREATE TABLE IF NOT EXISTS identities (
            id TEXT PRIMARY KEY,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            display_name TEXT,
            metadata_json TEXT,
            created_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            UNIQUE(gateway_id, external_id)
        );

        CREATE TABLE IF NOT EXISTS profiles (
            id TEXT PRIMARY KEY,
            display_name TEXT,
            summary TEXT,
            comm_style TEXT,
            first_seen INTEGER NOT NULL,
            last_seen INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS display_name_observations (
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
            WHERE source_message_id IS NOT NULL;

        CREATE TABLE IF NOT EXISTS persons (
            id TEXT PRIMARY KEY,
            display_name TEXT,
            summary TEXT,
            comm_style TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS profile_identities (
            profile_id TEXT NOT NULL,
            identity_id TEXT NOT NULL,
            status TEXT NOT NULL,
            confidence REAL NOT NULL,
            evidence_json TEXT,
            created_at INTEGER NOT NULL,
            removed_at INTEGER,
            PRIMARY KEY(profile_id, identity_id)
        );
        CREATE INDEX IF NOT EXISTS idx_profile_identities_identity ON profile_identities(identity_id, status);

        CREATE TABLE IF NOT EXISTS person_profiles (
            person_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            status TEXT NOT NULL,
            confidence REAL NOT NULL,
            evidence_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            detached_at INTEGER,
            PRIMARY KEY(person_id, profile_id)
        );
        CREATE INDEX IF NOT EXISTS idx_person_profiles_profile ON person_profiles(profile_id, status);

        CREATE TABLE IF NOT EXISTS identity_claims (
            id TEXT PRIMARY KEY,
            claimant_id TEXT NOT NULL,
            claimed_person_id TEXT NOT NULL,
            evidence TEXT NOT NULL,
            reason TEXT,
            evidence_json TEXT NOT NULL DEFAULT '{}',
            confidence REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_claims_status ON identity_claims(status);

        CREATE TABLE IF NOT EXISTS identity_disclosure_audits (
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
            ON identity_disclosure_audits(action_id);

        CREATE TABLE IF NOT EXISTS social_graph (
            person_a TEXT NOT NULL,
            person_b TEXT NOT NULL,
            relation TEXT NOT NULL,
            direction TEXT NOT NULL DEFAULT 'bidirectional',
            confidence REAL NOT NULL DEFAULT 0.5,
            status TEXT NOT NULL DEFAULT 'stated',
            evidence_json TEXT,
            source_kind TEXT NOT NULL DEFAULT 'system',
            asserted_by_person_id TEXT,
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(person_a, person_b, relation)
        );

        CREATE TABLE IF NOT EXISTS groups (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            context TEXT NOT NULL DEFAULT 'social',
            UNIQUE(gateway_id, external_id)
        );

        CREATE TABLE IF NOT EXISTS group_members (
            group_id TEXT NOT NULL,
            person_id TEXT NOT NULL,
            PRIMARY KEY(group_id, person_id)
        );

        CREATE TABLE IF NOT EXISTS behavior_directives (
            id TEXT PRIMARY KEY,
            scope_type TEXT NOT NULL,
            scope_value TEXT,
            directive TEXT NOT NULL,
            set_by TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0,
            active INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            expires_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_directives_scope ON behavior_directives(scope_type, scope_value);
        CREATE INDEX IF NOT EXISTS idx_directives_active ON behavior_directives(active);",
    )?;

    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(
            memory_id TEXT PRIMARY KEY,
            embedding float[{embedding_dims}]
        );"
    ))?;

    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            content,
            content_rowid='rowid'
        );",
    )?;

    record_clean_schema(conn)?;

    Ok(())
}
