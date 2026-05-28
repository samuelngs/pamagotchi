use super::*;

#[test]
fn init_schema_records_ordered_migrations() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    let migrations = conn
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        migrations,
        vec![
            (1, "memory_schema_v2".into()),
            (2, "message_source_and_reply_context".into()),
            (3, "action_transcripts_and_intents".into()),
            (4, "social_graph_metadata".into()),
            (5, "identity_claim_metadata".into()),
            (6, "thought_metadata".into()),
            (7, "conversation_summary_metadata".into()),
            (8, "identity_disclosure_audit".into()),
            (9, "outbound_delivery_audit".into()),
            (10, "event_inbox".into()),
            (11, "action_message_source_dedupe".into()),
            (12, "state_journal".into()),
            (13, "display_name_observations".into()),
            (14, "review_outputs".into()),
            (15, "action_prompt_snapshots".into()),
            (16, "intent_chosen_person_approval".into()),
            (17, "event_inbox_failure_error".into()),
            (18, "action_run_outcome_memory_artifacts".into()),
            (19, "social_graph_direction".into()),
        ]
    );
}

#[test]
fn init_schema_creates_current_message_source_uniqueness() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    assert!(index_exists(
        &conn,
        "messages",
        "idx_messages_source_unique"
    ));
}

#[test]
fn init_schema_migrates_representative_old_tables() {
    let conn = schema_test_conn();
    conn.execute_batch(
        "CREATE TABLE memories (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            source TEXT NOT NULL,
            importance REAL NOT NULL DEFAULT 0.5,
            sensitivity REAL NOT NULL DEFAULT 0.0,
            emotional_valence REAL NOT NULL DEFAULT 0.0,
            created_at INTEGER NOT NULL,
            accessed_at INTEGER NOT NULL,
            access_count INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]'
        );
        CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            identity_id TEXT,
            profile_id TEXT,
            person_id TEXT,
            metadata TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE conversations (
            id TEXT PRIMARY KEY,
            gateway_id TEXT,
            identity_id TEXT,
            profile_id TEXT,
            person_id TEXT,
            group_id TEXT,
            summary TEXT,
            started_at INTEGER NOT NULL,
            last_message_at INTEGER NOT NULL,
            message_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE social_graph (
            person_a TEXT NOT NULL,
            person_b TEXT NOT NULL,
            relation TEXT NOT NULL,
            PRIMARY KEY(person_a, person_b, relation)
        );
        CREATE TABLE identity_claims (
            id TEXT PRIMARY KEY,
            claimant_id TEXT NOT NULL,
            claimed_person_id TEXT NOT NULL,
            evidence TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );
        CREATE TABLE thoughts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            memories_accessed TEXT NOT NULL DEFAULT '[]',
            subjects TEXT NOT NULL DEFAULT '[]'
        );",
    )
    .unwrap();

    schema::init_schema(&conn, 4).unwrap();

    let memory_columns = table_columns(&conn, "memories");
    assert!(memory_columns.contains(&"memory_type".to_string()));
    assert!(memory_columns.contains(&"truth_status".to_string()));
    assert!(memory_columns.contains(&"dedupe_key".to_string()));

    let message_columns = table_columns(&conn, "messages");
    assert!(message_columns.contains(&"source_gateway_id".to_string()));
    assert!(message_columns.contains(&"source_message_id".to_string()));
    assert!(message_columns.contains(&"sender_external_id".to_string()));
    assert!(message_columns.contains(&"reply_external_id".to_string()));
    assert!(index_exists(
        &conn,
        "messages",
        "idx_messages_source_unique"
    ));

    let conversation_columns = table_columns(&conn, "conversations");
    assert!(conversation_columns.contains(&"summary_covered_message_ids".to_string()));
    assert!(conversation_columns.contains(&"summary_updated_at".to_string()));
    assert!(conversation_columns.contains(&"summary_version".to_string()));

    let social_columns = table_columns(&conn, "social_graph");
    assert!(social_columns.contains(&"direction".to_string()));
    assert!(social_columns.contains(&"confidence".to_string()));
    assert!(social_columns.contains(&"status".to_string()));
    assert!(social_columns.contains(&"evidence_json".to_string()));
    assert!(social_columns.contains(&"source_kind".to_string()));
    assert!(social_columns.contains(&"asserted_by_person_id".to_string()));
    assert!(social_columns.contains(&"created_at".to_string()));
    assert!(social_columns.contains(&"updated_at".to_string()));

    let claim_columns = table_columns(&conn, "identity_claims");
    assert!(claim_columns.contains(&"reason".to_string()));
    assert!(claim_columns.contains(&"evidence_json".to_string()));

    let thought_columns = table_columns(&conn, "thoughts");
    assert!(thought_columns.contains(&"importance".to_string()));
    assert!(thought_columns.contains(&"confidence".to_string()));
    assert!(thought_columns.contains(&"action_id".to_string()));

    let event_columns = table_columns(&conn, "event_inbox");
    assert!(event_columns.contains(&"last_error".to_string()));

    let action_run_columns = table_columns(&conn, "action_runs");
    assert!(action_run_columns.contains(&"memories_formed".to_string()));
    assert!(action_run_columns.contains(&"recalled_memory_ids".to_string()));

    for table in [
        "action_runs",
        "action_turns",
        "action_prompt_snapshots",
        "action_tool_calls",
        "action_messages",
        "action_outbound_deliveries",
        "action_review_watermarks",
        "intents",
        "event_inbox",
        "state_journal",
        "display_name_observations",
        "review_outputs",
        "identity_disclosure_audits",
    ] {
        assert!(
            table_exists(&conn, table),
            "expected migration to create {table}"
        );
    }
}

fn index_exists(conn: &rusqlite::Connection, table: &str, index: &str) -> bool {
    conn.prepare(&format!("PRAGMA index_list({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>("name"))
        .unwrap()
        .filter_map(|row| row.ok())
        .any(|name| name == index)
}

#[test]
fn init_schema_migrates_event_inbox_failure_error_column() {
    let conn = schema_test_conn();
    conn.execute_batch(
        "CREATE TABLE event_inbox (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            due_at INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            dedupe_key TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            fired_at INTEGER
        );",
    )
    .unwrap();

    schema::init_schema(&conn, 4).unwrap();

    let event_columns = table_columns(&conn, "event_inbox");
    assert!(event_columns.contains(&"last_error".to_string()));
}

#[test]
fn init_schema_migrates_action_run_outcome_memory_columns() {
    let conn = schema_test_conn();
    conn.execute_batch(
        "CREATE TABLE action_runs (
            action_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            task TEXT NOT NULL,
            conversation_id TEXT,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            status TEXT NOT NULL,
            responded INTEGER NOT NULL DEFAULT 0,
            attempts INTEGER NOT NULL DEFAULT 0
        );",
    )
    .unwrap();

    schema::init_schema(&conn, 4).unwrap();

    let columns = table_columns(&conn, "action_runs");
    assert!(columns.contains(&"memories_formed".to_string()));
    assert!(columns.contains(&"recalled_memory_ids".to_string()));
}

#[test]
fn init_schema_migrates_intent_chosen_person_approval_column() {
    let conn = schema_test_conn();
    conn.execute_batch(
        "CREATE TABLE intents (
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
            last_fired_at INTEGER
        );
        INSERT INTO intents (
            id, kind, status, task, priority, created_at, updated_at
        ) VALUES (
            'intent-old', 'scheduled', 'active', 'Old follow-up', 50, 1000, 1000
        );",
    )
    .unwrap();

    schema::init_schema(&conn, 4).unwrap();

    let columns = table_columns(&conn, "intents");
    assert!(columns.contains(&"chosen_person_approved".to_string()));
    let chosen_person_approved: i64 = conn
        .query_row(
            "SELECT chosen_person_approved FROM intents WHERE id = 'intent-old'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(chosen_person_approved, 0);
}
