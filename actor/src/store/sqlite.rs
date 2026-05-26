use super::{
    ActorSnapshot, ConversationSummary, Memory, MemoryKind, MemorySource, MemoryUpdate,
    MessageRole, RecallQuery, Store, StoredMessage, Thought, ThoughtKind,
};
use crate::identity::{
    ClaimEvidence, ClaimStatus, Group, GroupContext, Identity, IdentityClaim, Person, Relation,
    SocialRelation,
};
use crate::state::{Authority, BehaviorDirective, DirectiveScope};
use protocol::{ConversationId, GroupId, MemoryId, PersonId};
use rusqlite::{Connection, params};
use sqlite_vec::sqlite3_vec_init;
use std::collections::HashSet;
use std::sync::{Mutex, Once};

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

pub struct SqliteConfig {
    pub path: String,
    pub embedding_dimensions: usize,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            path: "actor.db".to_string(),
            embedding_dimensions: 1536,
        }
    }
}

static INIT_VEC: Once = Once::new();

fn register_sqlite_vec() {
    INIT_VEC.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
    });
}

impl SqliteStore {
    pub fn open(config: SqliteConfig) -> anyhow::Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open(&config.path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        init_schema(&conn, config.embedding_dimensions)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory(embedding_dimensions: usize) -> anyhow::Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        init_schema(&conn, embedding_dimensions)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> anyhow::Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("lock poisoned"))
    }
}

fn init_schema(conn: &Connection, embedding_dims: usize) -> anyhow::Result<()> {
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
            person_id TEXT,
            metadata TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, timestamp);

        CREATE TABLE IF NOT EXISTS memory_people (
            memory_id TEXT NOT NULL,
            person_id TEXT NOT NULL,
            PRIMARY KEY(memory_id, person_id)
        );

        CREATE TABLE IF NOT EXISTS thoughts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            memories_accessed TEXT NOT NULL DEFAULT '[]',
            people TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_thoughts_ts ON thoughts(timestamp);

        CREATE TABLE IF NOT EXISTS snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            saved_at INTEGER NOT NULL,
            data TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_snapshots_saved ON snapshots(saved_at);

        CREATE TABLE IF NOT EXISTS people (
            id TEXT PRIMARY KEY,
            name TEXT,
            summary TEXT,
            comm_style TEXT,
            first_seen INTEGER NOT NULL,
            last_seen INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS identities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            person_id TEXT NOT NULL,
            gateway_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            display_name TEXT,
            UNIQUE(gateway_id, external_id)
        );
        CREATE INDEX IF NOT EXISTS idx_identities_person ON identities(person_id);

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

fn build_fts_query(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().filter(|w| w.len() > 1).collect();
    if words.is_empty() {
        return input.to_string();
    }
    words
        .iter()
        .map(|w| {
            let clean: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{clean}\"")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

struct TxGuard<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> TxGuard<'a> {
    fn begin(conn: &'a Connection) -> anyhow::Result<Self> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self { conn, done: false })
    }

    fn commit(mut self) -> anyhow::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.done = true;
        Ok(())
    }
}

impl Drop for TxGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

fn read_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let id: String = row.get("id")?;
    let kind_str: String = row.get("kind")?;
    let source_json: String = row.get("source")?;
    let tags_json: String = row.get("tags")?;

    Ok(Memory {
        id: MemoryId(id),
        kind: MemoryKind::parse(&kind_str).unwrap_or(MemoryKind::Episodic),
        content: row.get("content")?,
        source: serde_json::from_str(&source_json).unwrap_or(MemorySource::External),
        importance: row.get("importance")?,
        sensitivity: row.get("sensitivity")?,
        emotional_valence: row.get("emotional_valence")?,
        created_at: row.get("created_at")?,
        accessed_at: row.get("accessed_at")?,
        access_count: row.get("access_count")?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        people: vec![],
        embedding: None,
    })
}

fn read_message(row: &rusqlite::Row) -> rusqlite::Result<StoredMessage> {
    let role_str: String = row.get("role")?;
    let metadata_json: String = row.get("metadata")?;
    let person_id: Option<String> = row.get("person_id")?;
    Ok(StoredMessage {
        timestamp: row.get("timestamp")?,
        role: MessageRole::parse(&role_str).unwrap_or(MessageRole::User),
        content: row.get("content")?,
        person: person_id.map(PersonId),
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
    })
}

#[async_trait::async_trait]
impl Store for SqliteStore {
    async fn save_snapshot(&self, snapshot: &ActorSnapshot) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let data = serde_json::to_string(snapshot)?;
        conn.execute(
            "INSERT INTO snapshots (saved_at, data) VALUES (?1, ?2)",
            params![snapshot.saved_at, data],
        )?;
        Ok(())
    }

    async fn load_latest_snapshot(&self) -> anyhow::Result<Option<ActorSnapshot>> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT data FROM snapshots ORDER BY saved_at DESC, id DESC LIMIT 1")?;
        match stmt.query_row([], |row| row.get::<_, String>(0)) {
            Ok(data) => Ok(Some(serde_json::from_str(&data)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn store_memory(&self, memory: &Memory) -> anyhow::Result<MemoryId> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        let source_json = serde_json::to_string(&memory.source)?;
        let tags_json = serde_json::to_string(&memory.tags)?;

        conn.execute(
            "INSERT INTO memories (id, kind, content, source, importance, sensitivity, emotional_valence,
             created_at, accessed_at, access_count, tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                memory.id.0,
                memory.kind.as_str(),
                memory.content,
                source_json,
                memory.importance,
                memory.sensitivity,
                memory.emotional_valence,
                memory.created_at,
                memory.accessed_at,
                memory.access_count,
                tags_json,
            ],
        )?;

        for person in &memory.people {
            conn.execute(
                "INSERT OR IGNORE INTO memory_people (memory_id, person_id) VALUES (?1, ?2)",
                params![memory.id.0, person.0],
            )?;
        }

        if let Some(ref embedding) = memory.embedding {
            let bytes = embedding_to_bytes(embedding);
            conn.execute(
                "INSERT INTO memories_vec (memory_id, embedding) VALUES (?1, ?2)",
                params![memory.id.0, bytes],
            )?;
        }

        conn.execute(
            "INSERT INTO memories_fts (rowid, content) VALUES ((SELECT rowid FROM memories WHERE id = ?1), ?2)",
            params![memory.id.0, memory.content],
        )?;

        tx.commit()?;
        Ok(memory.id.clone())
    }

    async fn get_memory(&self, id: &MemoryId) -> anyhow::Result<Option<Memory>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, content, source, importance, sensitivity, emotional_valence,
                    created_at, accessed_at, access_count, tags
             FROM memories WHERE id = ?1",
        )?;
        match stmt.query_row(params![id.0], read_memory) {
            Ok(mut memory) => {
                let mut people_stmt =
                    conn.prepare("SELECT person_id FROM memory_people WHERE memory_id = ?1")?;
                memory.people = people_stmt
                    .query_map(params![id.0], |row| Ok(PersonId(row.get::<_, String>(0)?)))?
                    .filter_map(|r| r.ok())
                    .collect();

                if let Ok(bytes) = conn.query_row(
                    "SELECT embedding FROM memories_vec WHERE memory_id = ?1",
                    params![id.0],
                    |row| row.get::<_, Vec<u8>>(0),
                ) {
                    memory.embedding = Some(bytes_to_embedding(&bytes));
                }
                Ok(Some(memory))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn update_memory(&self, id: &MemoryId, update: &MemoryUpdate) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;

        if let Some(ref content) = update.content {
            conn.execute(
                "UPDATE memories SET content = ?1 WHERE id = ?2",
                params![content, id.0],
            )?;
            conn.execute(
                "UPDATE memories_fts SET content = ?1 WHERE rowid = (SELECT rowid FROM memories WHERE id = ?2)",
                params![content, id.0],
            )?;
        }
        if let Some(importance) = update.importance {
            conn.execute(
                "UPDATE memories SET importance = ?1 WHERE id = ?2",
                params![importance, id.0],
            )?;
        }
        if let Some(sensitivity) = update.sensitivity {
            conn.execute(
                "UPDATE memories SET sensitivity = ?1 WHERE id = ?2",
                params![sensitivity, id.0],
            )?;
        }
        if let Some(valence) = update.emotional_valence {
            conn.execute(
                "UPDATE memories SET emotional_valence = ?1 WHERE id = ?2",
                params![valence, id.0],
            )?;
        }
        if let Some(ref tags) = update.tags {
            let tags_json = serde_json::to_string(tags)?;
            conn.execute(
                "UPDATE memories SET tags = ?1 WHERE id = ?2",
                params![tags_json, id.0],
            )?;
        }
        if let Some(ref people) = update.people {
            conn.execute(
                "DELETE FROM memory_people WHERE memory_id = ?1",
                params![id.0],
            )?;
            for person in people {
                conn.execute(
                    "INSERT OR IGNORE INTO memory_people (memory_id, person_id) VALUES (?1, ?2)",
                    params![id.0, person.0],
                )?;
            }
        }
        if let Some(ref embedding) = update.embedding {
            let bytes = embedding_to_bytes(embedding);
            conn.execute(
                "DELETE FROM memories_vec WHERE memory_id = ?1",
                params![id.0],
            )?;
            conn.execute(
                "INSERT INTO memories_vec (memory_id, embedding) VALUES (?1, ?2)",
                params![id.0, bytes],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    async fn recall(&self, query: &RecallQuery) -> anyhow::Result<Vec<Memory>> {
        let conn = self.lock()?;
        let fetch_limit = ((query.offset + query.limit) * 2) as i64;

        let person_filter = query.person.as_ref();
        let person_ids: HashSet<String> = if let Some(ref person) = person_filter {
            let mut stmt =
                conn.prepare("SELECT memory_id FROM memory_people WHERE person_id = ?1")?;
            stmt.query_map(params![person.0], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            HashSet::new()
        };

        let mut memories = if let Some(ref embedding) = query.embedding {
            let bytes = embedding_to_bytes(embedding);
            let mut stmt = conn.prepare(
                "SELECT m.id, m.kind, m.content, m.source, m.importance, m.sensitivity, m.emotional_valence,
                        m.created_at, m.accessed_at, m.access_count, m.tags
                 FROM (SELECT memory_id, distance FROM memories_vec WHERE embedding MATCH ?1 AND k = ?2) v
                 JOIN memories m ON m.id = v.memory_id
                 ORDER BY v.distance",
            )?;
            stmt.query_map(params![bytes, fetch_limit], read_memory)?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>()
        } else if let Some(ref text) = query.text {
            let fts_query = build_fts_query(text);
            let mut stmt = conn.prepare(
                "SELECT m.id, m.kind, m.content, m.source, m.importance, m.sensitivity, m.emotional_valence,
                        m.created_at, m.accessed_at, m.access_count, m.tags
                 FROM memories_fts f
                 JOIN memories m ON m.rowid = f.rowid
                 WHERE memories_fts MATCH ?1
                 ORDER BY bm25(memories_fts) ASC
                 LIMIT ?2",
            )?;
            let results: Vec<_> = stmt
                .query_map(params![fts_query, fetch_limit], read_memory)?
                .filter_map(|r| r.ok())
                .collect();
            if results.is_empty() {
                let escaped = text
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                let pattern = format!("%{escaped}%");
                let mut fallback = conn.prepare(
                    "SELECT id, kind, content, source, importance, sensitivity, emotional_valence,
                            created_at, accessed_at, access_count, tags
                     FROM memories WHERE content LIKE ?1 ESCAPE '\\'
                     ORDER BY importance DESC, created_at DESC
                     LIMIT ?2",
                )?;
                fallback
                    .query_map(params![pattern, fetch_limit], read_memory)?
                    .filter_map(|r| r.ok())
                    .collect::<Vec<_>>()
            } else {
                results
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, kind, content, source, importance, sensitivity, emotional_valence,
                        created_at, accessed_at, access_count, tags
                 FROM memories
                 ORDER BY importance DESC, created_at DESC
                 LIMIT ?1",
            )?;
            stmt.query_map(params![fetch_limit], read_memory)?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>()
        };

        memories.retain(|m| {
            if let Some(ref kind) = query.kind {
                if m.kind.as_str() != kind.as_str() {
                    return false;
                }
            }
            if let Some(min_imp) = query.min_importance {
                if m.importance < min_imp {
                    return false;
                }
            }
            if let Some(ref range) = query.time_range {
                if let Some(start) = range.start {
                    if m.created_at < start {
                        return false;
                    }
                }
                if let Some(end) = range.end {
                    if m.created_at > end {
                        return false;
                    }
                }
            }
            if let Some(max_sens) = query.max_sensitivity {
                if m.sensitivity > max_sens {
                    return false;
                }
            }
            if person_filter.is_some() && !person_ids.contains(&m.id.0) {
                return false;
            }
            true
        });

        if query.offset > 0 {
            memories.drain(..query.offset.min(memories.len()));
        }
        memories.truncate(query.limit);

        let mut people_stmt =
            conn.prepare("SELECT person_id FROM memory_people WHERE memory_id = ?1")?;
        let mut access_stmt = conn.prepare(
            "UPDATE memories SET accessed_at = unixepoch(), access_count = access_count + 1 WHERE id = ?1",
        )?;
        for memory in &mut memories {
            memory.people = people_stmt
                .query_map(params![memory.id.0], |row| {
                    Ok(PersonId(row.get::<_, String>(0)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            let _ = access_stmt.execute(params![memory.id.0]);
        }

        Ok(memories)
    }

    async fn forget(&self, id: &MemoryId) -> anyhow::Result<bool> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        conn.execute(
            "DELETE FROM memories_fts WHERE rowid = (SELECT rowid FROM memories WHERE id = ?1)",
            params![id.0],
        )?;
        conn.execute(
            "DELETE FROM memories_vec WHERE memory_id = ?1",
            params![id.0],
        )?;
        conn.execute(
            "DELETE FROM memory_people WHERE memory_id = ?1",
            params![id.0],
        )?;
        let rows = conn.execute("DELETE FROM memories WHERE id = ?1", params![id.0])?;
        tx.commit()?;
        Ok(rows > 0)
    }

    async fn append_message(
        &self,
        conv: &ConversationId,
        gateway_id: Option<&str>,
        group: Option<&GroupId>,
        msg: &StoredMessage,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        let person_id = msg.person.as_ref().map(|p| &p.0);
        let group_id = group.map(|g| &g.0);

        conn.execute(
            "INSERT INTO conversations (id, gateway_id, person_id, group_id, started_at, last_message_at, message_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, 1)
             ON CONFLICT(id) DO UPDATE SET
                last_message_at = ?5,
                message_count = message_count + 1,
                gateway_id = COALESCE(conversations.gateway_id, excluded.gateway_id),
                person_id = COALESCE(conversations.person_id, excluded.person_id),
                group_id = COALESCE(conversations.group_id, excluded.group_id)",
            params![conv.0, gateway_id, person_id, group_id, msg.timestamp],
        )?;

        let metadata_json = serde_json::to_string(&msg.metadata)?;
        conn.execute(
            "INSERT INTO messages (conversation_id, timestamp, role, content, person_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                conv.0,
                msg.timestamp,
                msg.role.as_str(),
                msg.content,
                person_id,
                metadata_json,
            ],
        )?;

        if let Some(person) = &msg.person {
            conn.execute(
                "UPDATE people SET last_seen = ?1 WHERE id = ?2 AND last_seen < ?1",
                params![msg.timestamp, person.0],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    async fn get_messages(
        &self,
        conv: &ConversationId,
        limit: usize,
        before: Option<i64>,
    ) -> anyhow::Result<Vec<StoredMessage>> {
        let conn = self.lock()?;

        let mut messages = if let Some(before_ts) = before {
            let mut stmt = conn.prepare(
                "SELECT timestamp, role, content, person_id, metadata
                 FROM messages
                 WHERE conversation_id = ?1 AND timestamp < ?2
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?3",
            )?;
            stmt.query_map(params![conv.0, before_ts, limit as i64], read_message)?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>()
        } else {
            let mut stmt = conn.prepare(
                "SELECT timestamp, role, content, person_id, metadata
                 FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?2",
            )?;
            stmt.query_map(params![conv.0, limit as i64], read_message)?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>()
        };

        messages.reverse();
        Ok(messages)
    }

    async fn list_conversations(&self) -> anyhow::Result<Vec<ConversationSummary>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, gateway_id, person_id, group_id, summary, message_count, started_at, last_message_at
             FROM conversations ORDER BY last_message_at DESC",
        )?;
        let results = stmt
            .query_map([], |row| {
                let person_id: Option<String> = row.get("person_id")?;
                let group_id: Option<String> = row.get("group_id")?;
                Ok(ConversationSummary {
                    id: ConversationId(row.get("id")?),
                    gateway_id: row.get("gateway_id")?,
                    person: person_id.map(PersonId),
                    group: group_id.map(GroupId),
                    summary: row.get("summary")?,
                    message_count: row.get("message_count")?,
                    started_at: row.get("started_at")?,
                    last_message_at: row.get("last_message_at")?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    async fn update_conversation_summary(
        &self,
        conv: &ConversationId,
        summary: &str,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE conversations SET summary = ?1 WHERE id = ?2",
            params![summary, conv.0],
        )?;
        Ok(())
    }

    async fn log_thought(&self, thought: &Thought) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let memories_json = serde_json::to_string(&thought.memories_accessed)?;
        let people_json = serde_json::to_string(&thought.people)?;
        conn.execute(
            "INSERT INTO thoughts (timestamp, kind, content, memories_accessed, people)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                thought.timestamp,
                thought.kind.as_str(),
                thought.content,
                memories_json,
                people_json,
            ],
        )?;
        Ok(())
    }

    async fn recent_thoughts(&self, limit: usize) -> anyhow::Result<Vec<Thought>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, kind, content, memories_accessed, people
             FROM thoughts ORDER BY timestamp DESC, id DESC LIMIT ?1",
        )?;
        let mut thoughts: Vec<Thought> = stmt
            .query_map(params![limit as i64], |row| {
                let kind_str: String = row.get("kind")?;
                let memories_json: String = row.get("memories_accessed")?;
                let people_json: String = row.get("people")?;
                Ok(Thought {
                    timestamp: row.get("timestamp")?,
                    kind: ThoughtKind::parse(&kind_str).unwrap_or(ThoughtKind::Observation),
                    content: row.get("content")?,
                    memories_accessed: serde_json::from_str(&memories_json).unwrap_or_default(),
                    people: serde_json::from_str(&people_json).unwrap_or_default(),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        thoughts.reverse();
        Ok(thoughts)
    }

    // People

    async fn add_person(&self, person: &Person) -> anyhow::Result<PersonId> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO people (id, name, summary, comm_style, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                person.id.0,
                person.name,
                person.summary,
                person.comm_style,
                person.first_seen,
                person.last_seen
            ],
        )?;
        Ok(person.id.clone())
    }

    async fn get_person(&self, id: &PersonId) -> anyhow::Result<Option<Person>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT id, name, summary, comm_style, first_seen, last_seen FROM people WHERE id = ?1",
            params![id.0],
            read_person,
        ) {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn update_person(
        &self,
        id: &PersonId,
        name: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        if let Some(name) = name {
            conn.execute(
                "UPDATE people SET name = ?1 WHERE id = ?2",
                params![name, id.0],
            )?;
        }
        if let Some(summary) = summary {
            conn.execute(
                "UPDATE people SET summary = ?1 WHERE id = ?2",
                params![summary, id.0],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn update_comm_style(&self, id: &PersonId, style: &str) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE people SET comm_style = ?1 WHERE id = ?2",
            params![style, id.0],
        )?;
        Ok(())
    }

    async fn touch_person(&self, id: &PersonId) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE people SET last_seen = unixepoch() WHERE id = ?1",
            params![id.0],
        )?;
        Ok(())
    }

    async fn list_people(&self) -> anyhow::Result<Vec<Person>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, summary, comm_style, first_seen, last_seen FROM people ORDER BY name",
        )?;
        let results = stmt
            .query_map([], read_person)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    // Identities

    async fn add_identity(&self, person: &PersonId, identity: &Identity) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO identities (person_id, gateway_id, external_id, display_name)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                person.0,
                identity.gateway_id,
                identity.external_id,
                identity.display_name.as_deref()
            ],
        )?;
        Ok(())
    }

    async fn resolve_identity(
        &self,
        gateway_id: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<Person>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT p.id, p.name, p.summary, p.comm_style, p.first_seen, p.last_seen
             FROM identities i JOIN people p ON p.id = i.person_id
             WHERE i.gateway_id = ?1 AND i.external_id = ?2",
            params![gateway_id, external_id],
            read_person,
        ) {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn get_identities(&self, person: &PersonId) -> anyhow::Result<Vec<Identity>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT gateway_id, external_id, display_name FROM identities WHERE person_id = ?1",
        )?;
        let results = stmt
            .query_map(params![person.0], |row| {
                Ok(Identity {
                    gateway_id: row.get("gateway_id")?,
                    external_id: row.get("external_id")?,
                    display_name: row.get::<_, Option<String>>("display_name")?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    async fn merge_people(&self, keep: &PersonId, merge: &PersonId) -> anyhow::Result<()> {
        if keep == merge {
            return Ok(());
        }
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        conn.execute(
            "UPDATE identities SET person_id = ?1 WHERE person_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE messages SET person_id = ?1 WHERE person_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE conversations SET person_id = ?1 WHERE person_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE OR IGNORE memory_people SET person_id = ?1 WHERE person_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "DELETE FROM memory_people WHERE person_id = ?1",
            params![merge.0],
        )?;
        conn.execute(
            "UPDATE behavior_directives SET set_by = ?1 WHERE set_by = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE behavior_directives SET scope_value = ?1 WHERE scope_type = 'person' AND scope_value = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE OR IGNORE group_members SET person_id = ?1 WHERE person_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "DELETE FROM group_members WHERE person_id = ?1",
            params![merge.0],
        )?;
        conn.execute(
            "UPDATE OR IGNORE social_graph SET person_a = ?1 WHERE person_a = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE OR IGNORE social_graph SET person_b = ?1 WHERE person_b = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute("DELETE FROM social_graph WHERE person_a = person_b", [])?;
        conn.execute(
            "DELETE FROM social_graph WHERE person_a = ?1 OR person_b = ?1",
            params![merge.0],
        )?;
        conn.execute(
            "UPDATE people SET first_seen = MIN(first_seen, (SELECT first_seen FROM people WHERE id = ?2)),
                             last_seen = MAX(last_seen, (SELECT last_seen FROM people WHERE id = ?2))
             WHERE id = ?1",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE identity_claims SET claimant_id = ?1 WHERE claimant_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE identity_claims SET claimed_person_id = ?1 WHERE claimed_person_id = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE memories SET source = json_set(source, '$.Conversation.person', ?1)
             WHERE json_extract(source, '$.Conversation.person') = ?2",
            params![keep.0, merge.0],
        )?;
        conn.execute(
            "UPDATE thoughts SET people = (
                SELECT json_group_array(val) FROM (
                    SELECT DISTINCT CASE WHEN j.value = ?2 THEN ?1 ELSE j.value END AS val
                    FROM json_each(thoughts.people) j
                )
            ) WHERE EXISTS (SELECT 1 FROM json_each(thoughts.people) WHERE value = ?2)",
            params![keep.0, merge.0],
        )?;
        conn.execute("DELETE FROM people WHERE id = ?1", params![merge.0])?;
        tx.commit()?;
        Ok(())
    }

    // Identity claims

    async fn create_claim(&self, claim: &IdentityClaim) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO identity_claims (id, claimant_id, claimed_person_id, evidence, confidence, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                claim.id,
                claim.claimant.0,
                claim.claimed_person.0,
                claim.evidence.as_str(),
                claim.confidence,
                claim.status.as_str(),
                claim.created_at,
            ],
        )?;
        Ok(())
    }

    async fn get_pending_claims(&self) -> anyhow::Result<Vec<IdentityClaim>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, claimant_id, claimed_person_id, evidence, confidence, status, created_at, resolved_at
             FROM identity_claims WHERE status = 'pending' ORDER BY created_at DESC",
        )?;
        let results = stmt
            .query_map([], read_claim)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    async fn resolve_claim(&self, claim_id: &str, status: &ClaimStatus) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        conn.execute(
            "UPDATE identity_claims SET status = ?1, resolved_at = ?2 WHERE id = ?3",
            params![status.as_str(), now, claim_id],
        )?;
        Ok(())
    }

    // Social graph

    async fn add_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO social_graph (person_a, person_b, relation) VALUES (?1, ?2, ?3)",
            params![a.0, b.0, relation.as_str()],
        )?;
        Ok(())
    }

    async fn get_relations(&self, person: &PersonId) -> anyhow::Result<Vec<SocialRelation>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT person_a, person_b, relation FROM social_graph
             WHERE person_a = ?1 OR person_b = ?1",
        )?;
        let results = stmt
            .query_map(params![person.0], |row| {
                let a: String = row.get("person_a")?;
                let b: String = row.get("person_b")?;
                let rel: String = row.get("relation")?;
                Ok(SocialRelation {
                    person_a: PersonId(a),
                    person_b: PersonId(b),
                    relation: Relation::parse(&rel),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    async fn remove_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM social_graph WHERE person_a = ?1 AND person_b = ?2 AND relation = ?3",
            params![a.0, b.0, relation.as_str()],
        )?;
        Ok(())
    }

    // Groups

    async fn add_group(&self, group: &Group) -> anyhow::Result<GroupId> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        conn.execute(
            "INSERT INTO groups (id, name, gateway_id, external_id, context) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                group.id.0,
                group.name,
                group.gateway_id,
                group.external_id,
                group.context.as_str(),
            ],
        )?;
        for member in &group.members {
            conn.execute(
                "INSERT OR IGNORE INTO group_members (group_id, person_id) VALUES (?1, ?2)",
                params![group.id.0, member.0],
            )?;
        }
        tx.commit()?;
        Ok(group.id.clone())
    }

    async fn get_group(&self, id: &GroupId) -> anyhow::Result<Option<Group>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT id, name, gateway_id, external_id, context FROM groups WHERE id = ?1",
            params![id.0],
            |row| {
                let context_str: String = row.get("context")?;
                Ok((
                    row.get::<_, String>("id")?,
                    row.get::<_, String>("name")?,
                    row.get::<_, String>("gateway_id")?,
                    row.get::<_, String>("external_id")?,
                    context_str,
                ))
            },
        ) {
            Ok((gid, name, gateway_id, external_id, context)) => {
                let mut stmt =
                    conn.prepare("SELECT person_id FROM group_members WHERE group_id = ?1")?;
                let members: Vec<PersonId> = stmt
                    .query_map(params![gid], |row| Ok(PersonId(row.get::<_, String>(0)?)))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(Some(Group {
                    id: GroupId(gid),
                    name,
                    gateway_id,
                    external_id,
                    context: GroupContext::parse(&context),
                    members,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn add_group_member(&self, group: &GroupId, person: &PersonId) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, person_id) VALUES (?1, ?2)",
            params![group.0, person.0],
        )?;
        Ok(())
    }

    async fn remove_group_member(&self, group: &GroupId, person: &PersonId) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND person_id = ?2",
            params![group.0, person.0],
        )?;
        Ok(())
    }

    // Behavior directives

    async fn add_directive(&self, directive: &BehaviorDirective) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO behavior_directives (id, scope_type, scope_value, directive, set_by, priority, active, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                directive.id,
                directive.scope.scope_type(),
                directive.scope.scope_value(),
                directive.directive,
                directive.set_by.0,
                directive.priority,
                directive.active as i32,
                directive.created_at,
                directive.expires_at,
            ],
        )?;
        Ok(())
    }

    async fn get_directives_for_context(
        &self,
        person: &PersonId,
        authority: &Authority,
        group: Option<&GroupId>,
    ) -> anyhow::Result<Vec<BehaviorDirective>> {
        let conn = self.lock()?;
        let mut results = Vec::new();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut stmt = conn.prepare(
            "SELECT id, scope_type, scope_value, directive, set_by, priority, active, created_at, expires_at
             FROM behavior_directives
             WHERE active = 1
               AND (expires_at IS NULL OR expires_at > ?4)
               AND (
                 scope_type = 'global'
                 OR (scope_type = 'person' AND scope_value = ?1)
                 OR (scope_type = 'authority' AND scope_value = ?2)
                 OR (scope_type = 'group' AND scope_value = ?3)
             )
             ORDER BY priority DESC",
        )?;

        let group_value: Option<&str> = group.map(|g| g.0.as_str());
        let rows = stmt.query_map(
            params![person.0, authority.as_str(), group_value, now],
            read_directive,
        )?;

        for row in rows {
            if let Ok(d) = row {
                results.push(d);
            }
        }

        Ok(results)
    }

    async fn update_directive(
        &self,
        id: &str,
        directive: Option<&str>,
        active: Option<bool>,
        priority: Option<i32>,
        expires_at: Option<Option<i64>>,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        if let Some(text) = directive {
            conn.execute(
                "UPDATE behavior_directives SET directive = ?1 WHERE id = ?2",
                params![text, id],
            )?;
        }
        if let Some(active) = active {
            conn.execute(
                "UPDATE behavior_directives SET active = ?1 WHERE id = ?2",
                params![active as i32, id],
            )?;
        }
        if let Some(priority) = priority {
            conn.execute(
                "UPDATE behavior_directives SET priority = ?1 WHERE id = ?2",
                params![priority, id],
            )?;
        }
        if let Some(expires) = expires_at {
            conn.execute(
                "UPDATE behavior_directives SET expires_at = ?1 WHERE id = ?2",
                params![expires, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn remove_directive(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.lock()?;
        let rows = conn.execute("DELETE FROM behavior_directives WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    async fn list_directives(&self) -> anyhow::Result<Vec<BehaviorDirective>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, scope_type, scope_value, directive, set_by, priority, active, created_at, expires_at
             FROM behavior_directives ORDER BY priority DESC, created_at DESC",
        )?;
        let results = stmt
            .query_map([], read_directive)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }
}

fn read_person(row: &rusqlite::Row) -> rusqlite::Result<Person> {
    Ok(Person {
        id: PersonId(row.get("id")?),
        name: row.get::<_, Option<String>>("name")?,
        summary: row.get::<_, Option<String>>("summary")?,
        comm_style: row.get::<_, Option<String>>("comm_style")?,
        first_seen: row.get("first_seen")?,
        last_seen: row.get("last_seen")?,
    })
}

fn read_claim(row: &rusqlite::Row) -> rusqlite::Result<IdentityClaim> {
    let evidence_str: String = row.get("evidence")?;
    let status_str: String = row.get("status")?;
    Ok(IdentityClaim {
        id: row.get("id")?,
        claimant: PersonId(row.get("claimant_id")?),
        claimed_person: PersonId(row.get("claimed_person_id")?),
        evidence: ClaimEvidence::parse(&evidence_str).unwrap_or(ClaimEvidence::SelfDeclaration),
        confidence: row.get("confidence")?,
        status: ClaimStatus::parse(&status_str).unwrap_or(ClaimStatus::Pending),
        created_at: row.get("created_at")?,
        resolved_at: row.get("resolved_at")?,
    })
}

fn read_directive(row: &rusqlite::Row) -> rusqlite::Result<BehaviorDirective> {
    let scope_type: String = row.get("scope_type")?;
    let scope_value: Option<String> = row.get("scope_value")?;
    let active: i32 = row.get("active")?;

    let scope = match scope_type.as_str() {
        "person" => DirectiveScope::Person(PersonId(scope_value.unwrap_or_default())),
        "authority" => DirectiveScope::Authority(
            Authority::parse(&scope_value.unwrap_or_default()).unwrap_or(Authority::Default),
        ),
        "group" => DirectiveScope::Group(GroupId(scope_value.unwrap_or_default())),
        _ => DirectiveScope::Global,
    };

    Ok(BehaviorDirective {
        id: row.get("id")?,
        scope,
        directive: row.get("directive")?,
        set_by: PersonId(row.get("set_by")?),
        priority: row.get("priority")?,
        active: active != 0,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ActorState, CoreTraits, GrowthConfig};

    fn test_store() -> SqliteStore {
        SqliteStore::open_in_memory(4).unwrap()
    }

    fn sample_memory(id: &str, content: &str, embedding: Vec<f32>) -> Memory {
        Memory {
            id: MemoryId(id.into()),
            kind: MemoryKind::Episodic,
            content: content.into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("conv-1".into()),
                person: PersonId("sam".into()),
            },
            importance: 0.8,
            sensitivity: 0.0,
            emotional_valence: -0.3,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec!["work".into()],
            people: vec![PersonId("sam".into())],
            embedding: Some(embedding),
        }
    }

    #[tokio::test]
    async fn memory_store_and_recall_by_text() {
        let store = test_store();
        let mem = sample_memory(
            "m1",
            "deployment incident was stressful",
            vec![0.1, 0.2, 0.3, 0.4],
        );
        store.store_memory(&mem).await.unwrap();

        let results = store
            .recall(&RecallQuery::by_text("deployment", 10))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "m1");
    }

    #[tokio::test]
    async fn memory_recall_by_embedding() {
        let store = test_store();
        store
            .store_memory(&sample_memory("m1", "first", vec![1.0, 0.0, 0.0, 0.0]))
            .await
            .unwrap();
        store
            .store_memory(&sample_memory("m2", "second", vec![0.0, 1.0, 0.0, 0.0]))
            .await
            .unwrap();

        let results = store
            .recall(&RecallQuery::by_embedding(vec![0.9, 0.1, 0.0, 0.0], 1))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "m1");
    }

    #[tokio::test]
    async fn memory_get_loads_embedding() {
        let store = test_store();
        store
            .store_memory(&sample_memory("m1", "test", vec![0.1, 0.2, 0.3, 0.4]))
            .await
            .unwrap();

        let loaded = store
            .get_memory(&MemoryId("m1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.embedding.unwrap(), vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[tokio::test]
    async fn memory_forget() {
        let store = test_store();
        store
            .store_memory(&sample_memory("m1", "gone", vec![0.1, 0.2, 0.3, 0.4]))
            .await
            .unwrap();

        assert!(store.forget(&MemoryId("m1".into())).await.unwrap());
        assert!(
            store
                .get_memory(&MemoryId("m1".into()))
                .await
                .unwrap()
                .is_none()
        );
        assert!(!store.forget(&MemoryId("m1".into())).await.unwrap());
    }

    #[tokio::test]
    async fn conversation_messages() {
        let store = test_store();
        let conv = ConversationId("c1".into());

        store
            .append_message(
                &conv,
                None,
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "hello".into(),
                    person: Some(PersonId("sam".into())),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();

        store
            .append_message(
                &conv,
                None,
                None,
                &StoredMessage {
                    timestamp: 1001,
                    role: MessageRole::Assistant,
                    content: "hi there".into(),
                    person: None,
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();

        let msgs = store.get_messages(&conv, 10, None).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].content, "hi there");

        let convs = store.list_conversations().await.unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].message_count, 2);
    }

    #[tokio::test]
    async fn thoughts() {
        let store = test_store();
        store
            .log_thought(&Thought {
                timestamp: 2000,
                kind: ThoughtKind::Reflection,
                content: "Sam seemed stressed".into(),
                memories_accessed: vec![MemoryId("m1".into())],
                people: vec![PersonId("sam".into())],
            })
            .await
            .unwrap();

        let thoughts = store.recent_thoughts(5).await.unwrap();
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0].content, "Sam seemed stressed");
    }

    #[tokio::test]
    async fn snapshots() {
        let store = test_store();
        let snapshot = ActorSnapshot {
            state: ActorState::new(CoreTraits::default()),
            config: GrowthConfig::default(),
            saved_at: 3000,
        };
        store.save_snapshot(&snapshot).await.unwrap();

        let loaded = store.load_latest_snapshot().await.unwrap().unwrap();
        assert_eq!(loaded.saved_at, 3000);
    }

    #[tokio::test]
    async fn recall_filters() {
        let store = test_store();
        store
            .store_memory(&Memory {
                id: MemoryId("m1".into()),
                kind: MemoryKind::Episodic,
                content: "episodic thing".into(),
                source: MemorySource::Reflection,
                importance: 0.9,
                sensitivity: 0.0,
                emotional_valence: 0.0,
                created_at: 1000,
                accessed_at: 1000,
                access_count: 0,
                tags: vec![],
                people: vec![],
                embedding: None,
            })
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("m2".into()),
                kind: MemoryKind::Semantic,
                content: "semantic fact".into(),
                source: MemorySource::Reflection,
                importance: 0.3,
                sensitivity: 0.0,
                emotional_valence: 0.0,
                created_at: 1000,
                accessed_at: 1000,
                access_count: 0,
                tags: vec![],
                people: vec![],
                embedding: None,
            })
            .await
            .unwrap();

        let results = store
            .recall(&RecallQuery::by_text("thing", 10).with_kind(MemoryKind::Episodic))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "m1");

        let results = store
            .recall(&RecallQuery::by_text("", 10).with_min_importance(0.5))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "m1");
    }

    fn sample_person(id: &str, name: &str) -> Person {
        Person {
            id: PersonId(id.into()),
            name: Some(name.into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        }
    }

    #[tokio::test]
    async fn people_crud() {
        let store = test_store();
        store
            .add_person(&sample_person("p1", "Alice"))
            .await
            .unwrap();
        store.add_person(&sample_person("p2", "Bob")).await.unwrap();

        let alice = store
            .get_person(&PersonId("p1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(alice.name, Some("Alice".into()));

        store
            .update_person(&PersonId("p1".into()), None, Some("likes cats"))
            .await
            .unwrap();
        let alice = store
            .get_person(&PersonId("p1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(alice.summary, Some("likes cats".into()));

        let all = store.list_people().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn identity_resolution() {
        let store = test_store();
        store
            .add_person(&sample_person("p1", "Alice"))
            .await
            .unwrap();
        store
            .add_identity(
                &PersonId("p1".into()),
                &Identity {
                    gateway_id: "discord".into(),
                    external_id: "discord-123".into(),
                    display_name: Some("alice#1234".into()),
                },
            )
            .await
            .unwrap();

        let found = store
            .resolve_identity("discord", "discord-123")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id.0, "p1");

        let not_found = store.resolve_identity("telegram", "unknown").await.unwrap();
        assert!(not_found.is_none());

        let identities = store.get_identities(&PersonId("p1".into())).await.unwrap();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].display_name.as_deref(), Some("alice#1234"));
    }

    #[tokio::test]
    async fn identity_claims() {
        let store = test_store();
        store
            .add_person(&sample_person("p1", "Alice Discord"))
            .await
            .unwrap();
        store
            .add_person(&sample_person("p2", "Alice Telegram"))
            .await
            .unwrap();

        store
            .create_claim(&IdentityClaim {
                id: "claim-1".into(),
                claimant: PersonId("p2".into()),
                claimed_person: PersonId("p1".into()),
                evidence: ClaimEvidence::SelfDeclaration,
                confidence: 0.1,
                status: ClaimStatus::Pending,
                created_at: 1000,
                resolved_at: None,
            })
            .await
            .unwrap();

        let pending = store.get_pending_claims().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "claim-1");

        store
            .resolve_claim("claim-1", &ClaimStatus::Confirmed)
            .await
            .unwrap();
        let pending = store.get_pending_claims().await.unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn merge_people_reconnects_data() {
        let store = test_store();
        store
            .add_person(&sample_person("p1", "Alice"))
            .await
            .unwrap();
        store
            .add_person(&sample_person("p2", "Alice Alt"))
            .await
            .unwrap();

        store
            .add_identity(
                &PersonId("p2".into()),
                &Identity {
                    gateway_id: "telegram".into(),
                    external_id: "tg-alice".into(),
                    display_name: Some("alice_t".into()),
                },
            )
            .await
            .unwrap();

        let conv = ConversationId("c1".into());
        store
            .append_message(
                &conv,
                None,
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "from alt account".into(),
                    person: Some(PersonId("p2".into())),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();

        store
            .merge_people(&PersonId("p1".into()), &PersonId("p2".into()))
            .await
            .unwrap();

        let resolved = store
            .resolve_identity("telegram", "tg-alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.id.0, "p1");

        assert!(
            store
                .get_person(&PersonId("p2".into()))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn social_graph() {
        let store = test_store();
        store.add_person(&sample_person("p1", "Sam")).await.unwrap();
        store.add_person(&sample_person("p2", "Mom")).await.unwrap();

        store
            .add_relation(
                &PersonId("p2".into()),
                &PersonId("p1".into()),
                &Relation::Parent,
            )
            .await
            .unwrap();

        let rels = store.get_relations(&PersonId("p1".into())).await.unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relation.as_str(), "parent");

        store
            .remove_relation(
                &PersonId("p2".into()),
                &PersonId("p1".into()),
                &Relation::Parent,
            )
            .await
            .unwrap();
        let rels = store.get_relations(&PersonId("p1".into())).await.unwrap();
        assert_eq!(rels.len(), 0);
    }

    #[tokio::test]
    async fn groups() {
        let store = test_store();
        store.add_person(&sample_person("p1", "Sam")).await.unwrap();
        store.add_person(&sample_person("p2", "Mom")).await.unwrap();

        store
            .add_group(&Group {
                id: GroupId("g1".into()),
                name: "Family Chat".into(),
                gateway_id: "discord".into(),
                external_id: "discord-family".into(),
                context: GroupContext::Family,
                members: vec![PersonId("p1".into()), PersonId("p2".into())],
            })
            .await
            .unwrap();

        let group = store
            .get_group(&GroupId("g1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(group.name, "Family Chat");
        assert_eq!(group.members.len(), 2);

        store
            .add_person(&sample_person("p3", "Sister"))
            .await
            .unwrap();
        store
            .add_group_member(&GroupId("g1".into()), &PersonId("p3".into()))
            .await
            .unwrap();

        let group = store
            .get_group(&GroupId("g1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(group.members.len(), 3);

        store
            .remove_group_member(&GroupId("g1".into()), &PersonId("p3".into()))
            .await
            .unwrap();
        let group = store
            .get_group(&GroupId("g1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(group.members.len(), 2);
    }

    #[tokio::test]
    async fn memory_people_association() {
        let store = test_store();
        let mem = Memory {
            id: MemoryId("m1".into()),
            kind: MemoryKind::Episodic,
            content: "Alice told me Bob got a new job".into(),
            source: MemorySource::Conversation {
                conversation_id: ConversationId("c1".into()),
                person: PersonId("alice".into()),
            },
            importance: 0.7,
            sensitivity: 0.5,
            emotional_valence: 0.3,
            created_at: 1000,
            accessed_at: 1000,
            access_count: 0,
            tags: vec![],
            people: vec![PersonId("alice".into()), PersonId("bob".into())],
            embedding: None,
        };
        store.store_memory(&mem).await.unwrap();

        let loaded = store
            .get_memory(&MemoryId("m1".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.people.len(), 2);

        let results = store
            .recall(&RecallQuery::by_text("Bob", 10).with_person(PersonId("bob".into())))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        let results = store
            .recall(&RecallQuery::by_text("Bob", 10).with_person(PersonId("charlie".into())))
            .await
            .unwrap();
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn behavior_directives() {
        let store = test_store();
        let sam = PersonId("sam".into());
        let mom = PersonId("mom".into());

        store
            .add_directive(&BehaviorDirective {
                id: "d1".into(),
                scope: DirectiveScope::Global,
                directive: "Never share private info between people".into(),
                set_by: sam.clone(),
                priority: 0,
                active: true,
                created_at: 1000,
                expires_at: None,
            })
            .await
            .unwrap();

        store
            .add_directive(&BehaviorDirective {
                id: "d2".into(),
                scope: DirectiveScope::Person(mom.clone()),
                directive: "Be polite, no crude humor".into(),
                set_by: sam.clone(),
                priority: 10,
                active: true,
                created_at: 1000,
                expires_at: None,
            })
            .await
            .unwrap();

        store
            .add_directive(&BehaviorDirective {
                id: "d3".into(),
                scope: DirectiveScope::Authority(Authority::Default),
                directive: "Be warm and respectful".into(),
                set_by: sam.clone(),
                priority: 5,
                active: true,
                created_at: 1000,
                expires_at: None,
            })
            .await
            .unwrap();

        let directives = store
            .get_directives_for_context(&mom, &Authority::Default, None)
            .await
            .unwrap();
        assert_eq!(directives.len(), 3);
        assert_eq!(directives[0].id, "d2");
        assert_eq!(directives[1].id, "d3");
        assert_eq!(directives[2].id, "d1");

        store
            .update_directive("d2", None, Some(false), None, None)
            .await
            .unwrap();
        let directives = store
            .get_directives_for_context(&mom, &Authority::Default, None)
            .await
            .unwrap();
        assert_eq!(directives.len(), 2);

        assert!(store.remove_directive("d1").await.unwrap());
        assert!(!store.remove_directive("nonexistent").await.unwrap());

        let all = store.list_directives().await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
