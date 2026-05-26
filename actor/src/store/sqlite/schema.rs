use rusqlite::Connection;

pub(super) fn init_schema(conn: &Connection, embedding_dims: usize) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
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
            metadata TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, timestamp);

        CREATE TABLE IF NOT EXISTS memory_subjects (
            memory_id TEXT NOT NULL,
            subject_type TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            role TEXT,
            confidence REAL NOT NULL,
            PRIMARY KEY(memory_id, subject_type, subject_id, role)
        );
        CREATE INDEX IF NOT EXISTS idx_memory_subjects_subject ON memory_subjects(subject_type, subject_id);

        CREATE TABLE IF NOT EXISTS thoughts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            memories_accessed TEXT NOT NULL DEFAULT '[]',
            subjects TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_thoughts_ts ON thoughts(timestamp);

        CREATE TABLE IF NOT EXISTS snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            saved_at INTEGER NOT NULL,
            data TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_snapshots_saved ON snapshots(saved_at);

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
            confidence REAL NOT NULL DEFAULT 0.0,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_claims_status ON identity_claims(status);

        CREATE TABLE IF NOT EXISTS social_graph (
            person_a TEXT NOT NULL,
            person_b TEXT NOT NULL,
            relation TEXT NOT NULL,
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

    Ok(())
}
