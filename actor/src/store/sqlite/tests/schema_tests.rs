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
fn init_schema_creates_chosen_human_intent_approval_column_only() {
    let conn = schema_test_conn();
    schema::init_schema(&conn, 4).unwrap();

    let columns = table_columns(&conn, "intents");
    assert!(columns.contains(&"chosen_human_approved".to_string()));
}

fn index_exists(conn: &rusqlite::Connection, table: &str, index: &str) -> bool {
    conn.prepare(&format!("PRAGMA index_list({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>("name"))
        .unwrap()
        .filter_map(|row| row.ok())
        .any(|name| name == index)
}
