use super::*;

pub(super) fn test_store() -> SqliteStore {
    SqliteStore::open_in_memory(4).unwrap()
}

pub(super) fn schema_test_conn() -> rusqlite::Connection {
    register_sqlite_vec();
    rusqlite::Connection::open_in_memory().unwrap()
}

pub(super) fn sample_memory(id: &str, content: &str, embedding: Vec<f32>) -> Memory {
    Memory {
        id: MemoryId(id.into()),
        kind: MemoryKind::Episodic,
        content: content.into(),
        source: MemorySource::Conversation {
            conversation_id: ConversationId("conv-1".into()),
            identity_id: None,
            profile_id: Some(ProfileId("profile-sam".into())),
            person_id: Some(PersonId("sam".into())),
            message_id: None,
        },
        importance: 0.8,
        sensitivity: 0.0,
        emotional_valence: -0.3,
        created_at: 1000,
        accessed_at: 1000,
        access_count: 0,
        tags: vec!["work".into()],
        subjects: vec![MemorySubject::profile(
            ProfileId("profile-sam".into()),
            Some("speaker".into()),
            1.0,
        )],
        embedding: Some(embedding),
        ..Memory::default()
    }
}

pub(super) fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>("name"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

pub(super) fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .unwrap()
}

pub(super) fn sample_person(id: &str, name: &str) -> Person {
    Person {
        id: PersonId(id.into()),
        name: Some(name.into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
    }
}

pub(super) fn sample_identity(
    id: &str,
    gateway_id: &str,
    external_id: &str,
    display_name: &str,
) -> Identity {
    Identity {
        id: IdentityId(id.into()),
        gateway_id: gateway_id.into(),
        external_id: external_id.into(),
        display_name: Some(display_name.into()),
        metadata: None,
        created_at: 1000,
        last_seen_at: 1000,
    }
}

pub(super) fn sample_profile(id: &str, display_name: &str) -> Profile {
    Profile {
        id: ProfileId(id.into()),
        display_name: Some(display_name.into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
        created_at: 1000,
        updated_at: 1000,
    }
}
