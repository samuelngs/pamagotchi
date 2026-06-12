use super::*;

#[test]
fn init_schema_records_clean_v1_schema() {
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

    assert_eq!(migrations, vec![(1, "clean_v1_schema".into())]);
}

#[test]
fn init_schema_creates_current_tables_and_indexes() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    for table in [
        "memories",
        "gateways",
        "spaces",
        "channels",
        "channel_memberships",
        "identity_conflicts",
        "identity_conflict_identities",
        "identity_conflict_profiles",
        "conversation_summary_coverage",
        "outbound_deliveries",
        "conversations",
        "messages",
        "memory_subjects",
        "memory_mutations",
        "thoughts",
        "action_runs",
        "action_turns",
        "action_prompt_snapshots",
        "action_tool_calls",
        "action_messages",
        "action_outbound_deliveries",
        "action_review_watermarks",
        "review_outputs",
        "intents",
        "event_inbox",
        "snapshots",
        "state_journal",
        "identities",
        "profiles",
        "display_name_observations",
        "persons",
        "profile_identities",
        "person_profiles",
        "identity_claims",
        "identity_disclosure_audits",
        "social_graph",
        "groups",
        "group_members",
        "behavior_directives",
    ] {
        assert!(table_exists(&conn, table), "expected table {table}");
    }

    assert!(index_exists(&conn, "messages", "idx_messages_conv"));
    assert!(index_exists(&conn, "messages", "idx_messages_message_id"));
    assert!(index_exists(
        &conn,
        "messages",
        "idx_messages_source_unique"
    ));
    assert!(index_exists(
        &conn,
        "action_messages",
        "idx_action_messages_source_unique"
    ));
    assert!(index_exists(&conn, "intents", "idx_intents_due"));
    assert!(index_exists(&conn, "event_inbox", "idx_event_inbox_due"));
}

#[test]
fn init_schema_rebuilds_memory_vector_index_when_embedding_dimensions_change() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();
    conn.execute(
        "INSERT INTO memories (
            id,
            kind,
            content,
            source,
            emotional_valence,
            created_at,
            accessed_at,
            embedding_model,
            embedding_version
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            "memory-1",
            "episodic",
            "Sam likes concise updates",
            "conversation",
            0.0,
            1000,
            1000,
            "embed-4",
            "v1"
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories_vec (memory_id, embedding) VALUES (?1, ?2)",
        params!["memory-1", test_embedding_bytes(&[0.1, 0.2, 0.3, 0.4])],
    )
    .unwrap();

    schema::init_schema(&conn, 3).unwrap();

    let create_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_vec'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(create_sql.contains("embedding float[3]"));

    let memory_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM memories WHERE id = ?1",
            params!["memory-1"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(memory_count, 1);

    let (embedding_model, embedding_version): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT embedding_model, embedding_version FROM memories WHERE id = ?1",
            params!["memory-1"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(embedding_model, None);
    assert_eq!(embedding_version, None);

    let vector_count: u32 = conn
        .query_row("SELECT count(*) FROM memories_vec", [], |row| row.get(0))
        .unwrap();
    assert_eq!(vector_count, 0);

    conn.execute(
        "INSERT INTO memories_vec (memory_id, embedding) VALUES (?1, ?2)",
        params!["memory-1", test_embedding_bytes(&[0.1, 0.2, 0.3])],
    )
    .unwrap();
}

#[test]
fn init_schema_creates_chosen_human_intent_approval_column_only() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    let columns = table_columns(&conn, "intents");
    assert!(columns.contains(&"chosen_human_approved".to_string()));
}

#[test]
fn conversations_table_keeps_channel_history_not_sender_snapshots() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    let columns = table_columns(&conn, "conversations");
    assert!(columns.contains(&"channel_id".to_string()));
    assert!(column_not_null(&conn, "conversations", "channel_id"));
    for removed in [
        "gateway_id",
        "identity_id",
        "profile_id",
        "person_id",
        "group_id",
    ] {
        assert!(
            !columns.contains(&removed.to_string()),
            "conversations should not keep {removed}"
        );
    }
}

#[test]
fn messages_table_stores_canonical_message_identity_and_channel() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    let columns = table_columns(&conn, "messages");
    for expected in ["message_id", "channel_id", "direction"] {
        assert!(
            columns.contains(&expected.to_string()),
            "messages should store {expected}"
        );
        assert!(column_not_null(&conn, "messages", expected));
    }
}

fn column_not_null(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>("name")?, row.get::<_, i64>("notnull")?))
        })
        .unwrap()
        .filter_map(|row| row.ok())
        .any(|(name, notnull)| name == column && notnull != 0)
}

fn index_exists(conn: &rusqlite::Connection, table: &str, index: &str) -> bool {
    conn.prepare(&format!("PRAGMA index_list({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>("name"))
        .unwrap()
        .filter_map(|row| row.ok())
        .any(|name| name == index)
}

fn test_embedding_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}
