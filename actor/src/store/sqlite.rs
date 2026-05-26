use super::{
    ActorSnapshot, ConversationSummary, Memory, MemoryKind, MemorySource, MemorySubject,
    MemorySubjectType, MemoryUpdate, MessageRole, RecallQuery, Store, StoredMessage, Thought,
    ThoughtKind,
};
use crate::identity::{
    ClaimStatus, Group, GroupContext, Identity, IdentityClaim, Person, PersonProfileLink,
    PersonProfileStatus, Profile, ProfileIdentityLink, Relation, ResolvedActorIdentity,
    SocialRelation,
};
use crate::state::{Authority, BehaviorDirective};
use protocol::{ConversationId, GroupId, IdentityId, MemoryId, PersonId, ProfileId};
use rusqlite::{Connection, OptionalExtension, params};
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

mod rows;
mod schema;
mod support;

#[cfg(test)]
mod tests;

use rows::*;
use schema::init_schema;
use support::{TxGuard, build_fts_query, bytes_to_embedding, embedding_to_bytes};
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

        for subject in &memory.subjects {
            conn.execute(
                "INSERT OR IGNORE INTO memory_subjects (memory_id, subject_type, subject_id, role, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    memory.id.0,
                    subject.subject_type.as_str(),
                    subject.subject_id,
                    subject.role,
                    subject.confidence,
                ],
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
                let mut subjects_stmt = conn.prepare(
                    "SELECT subject_type, subject_id, role, confidence FROM memory_subjects WHERE memory_id = ?1",
                )?;
                memory.subjects = subjects_stmt
                    .query_map(params![id.0], |row| {
                        let subject_type: String = row.get(0)?;
                        Ok(MemorySubject {
                            subject_type: MemorySubjectType::parse(&subject_type)
                                .unwrap_or(MemorySubjectType::Profile),
                            subject_id: row.get(1)?,
                            role: row.get(2)?,
                            confidence: row.get(3)?,
                        })
                    })?
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
        if let Some(ref subjects) = update.subjects {
            conn.execute(
                "DELETE FROM memory_subjects WHERE memory_id = ?1",
                params![id.0],
            )?;
            for subject in subjects {
                conn.execute(
                    "INSERT OR IGNORE INTO memory_subjects (memory_id, subject_type, subject_id, role, confidence)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id.0,
                        subject.subject_type.as_str(),
                        subject.subject_id,
                        subject.role,
                        subject.confidence,
                    ],
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

        let mut subject_filters = Vec::new();
        if let Some(identity) = query.identity.as_ref() {
            subject_filters.push(("identity", identity.0.as_str()));
        }
        if let Some(profile) = query.profile.as_ref() {
            subject_filters.push(("profile", profile.0.as_str()));
        }
        if let Some(person) = query.person.as_ref() {
            subject_filters.push(("person", person.0.as_str()));
        }
        let subject_ids: HashSet<String> = if subject_filters.is_empty() {
            HashSet::new()
        } else {
            let mut ids = HashSet::new();
            let mut stmt = conn.prepare(
                "SELECT memory_id FROM memory_subjects WHERE subject_type = ?1 AND subject_id = ?2",
            )?;
            for (subject_type, subject_id) in &subject_filters {
                for id in stmt
                    .query_map(params![subject_type, subject_id], |row| {
                        row.get::<_, String>(0)
                    })?
                    .filter_map(|r| r.ok())
                {
                    ids.insert(id);
                }
            }
            ids
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
                 ORDER BY m.created_at DESC, bm25(memories_fts) ASC
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
                     ORDER BY created_at DESC, importance DESC
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
                 ORDER BY created_at DESC, importance DESC
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
            if !subject_filters.is_empty() && !subject_ids.contains(&m.id.0) {
                return false;
            }
            true
        });

        memories.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.importance.total_cmp(&a.importance))
        });

        if query.offset > 0 {
            memories.drain(..query.offset.min(memories.len()));
        }
        memories.truncate(query.limit);

        let mut subjects_stmt = conn.prepare(
            "SELECT subject_type, subject_id, role, confidence FROM memory_subjects WHERE memory_id = ?1",
        )?;
        let mut access_stmt = conn.prepare(
            "UPDATE memories SET accessed_at = unixepoch(), access_count = access_count + 1 WHERE id = ?1",
        )?;
        for memory in &mut memories {
            memory.subjects = subjects_stmt
                .query_map(params![memory.id.0], |row| {
                    let subject_type: String = row.get(0)?;
                    Ok(MemorySubject {
                        subject_type: MemorySubjectType::parse(&subject_type)
                            .unwrap_or(MemorySubjectType::Profile),
                        subject_id: row.get(1)?,
                        role: row.get(2)?,
                        confidence: row.get(3)?,
                    })
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
            "DELETE FROM memory_subjects WHERE memory_id = ?1",
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
        let identity_id = msg.identity.as_ref().map(|p| &p.0);
        let profile_id = msg.profile.as_ref().map(|p| &p.0);
        let person_id = msg.person.as_ref().map(|p| &p.0);
        let group_id = group.map(|g| &g.0);

        conn.execute(
            "INSERT INTO conversations (id, gateway_id, identity_id, profile_id, person_id, group_id, started_at, last_message_at, message_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 1)
             ON CONFLICT(id) DO UPDATE SET
                last_message_at = ?7,
                message_count = message_count + 1,
                gateway_id = COALESCE(conversations.gateway_id, excluded.gateway_id),
                identity_id = COALESCE(excluded.identity_id, conversations.identity_id),
                profile_id = COALESCE(excluded.profile_id, conversations.profile_id),
                person_id = COALESCE(conversations.person_id, excluded.person_id),
                group_id = COALESCE(conversations.group_id, excluded.group_id)",
            params![
                conv.0,
                gateway_id,
                identity_id,
                profile_id,
                person_id,
                group_id,
                msg.timestamp,
            ],
        )?;

        let metadata_json = serde_json::to_string(&msg.metadata)?;
        conn.execute(
            "INSERT INTO messages (conversation_id, timestamp, role, content, identity_id, profile_id, person_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                conv.0,
                msg.timestamp,
                msg.role.as_str(),
                msg.content,
                identity_id,
                profile_id,
                person_id,
                metadata_json,
            ],
        )?;

        if let Some(profile) = &msg.profile {
            conn.execute(
                "UPDATE profiles SET last_seen = ?1, updated_at = ?1 WHERE id = ?2 AND last_seen < ?1",
                params![msg.timestamp, profile.0],
            )?;
        }
        if let Some(person) = &msg.person {
            conn.execute(
                "UPDATE persons SET updated_at = ?1 WHERE id = ?2 AND updated_at < ?1",
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
                "SELECT timestamp, role, content, identity_id, profile_id, person_id, metadata
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
                "SELECT timestamp, role, content, identity_id, profile_id, person_id, metadata
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
            "SELECT id, gateway_id, identity_id, profile_id, person_id, group_id, summary, message_count, started_at, last_message_at
             FROM conversations ORDER BY last_message_at DESC",
        )?;
        let results = stmt
            .query_map([], |row| {
                let identity_id: Option<String> = row.get("identity_id")?;
                let profile_id: Option<String> = row.get("profile_id")?;
                let person_id: Option<String> = row.get("person_id")?;
                let group_id: Option<String> = row.get("group_id")?;
                Ok(ConversationSummary {
                    id: ConversationId(row.get("id")?),
                    gateway_id: row.get("gateway_id")?,
                    identity: identity_id.map(IdentityId),
                    profile: profile_id.map(ProfileId),
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
        let subjects_json = serde_json::to_string(&thought.subjects)?;
        conn.execute(
            "INSERT INTO thoughts (timestamp, kind, content, memories_accessed, subjects)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                thought.timestamp,
                thought.kind.as_str(),
                thought.content,
                memories_json,
                subjects_json,
            ],
        )?;
        Ok(())
    }

    async fn recent_thoughts(&self, limit: usize) -> anyhow::Result<Vec<Thought>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, kind, content, memories_accessed, subjects
             FROM thoughts ORDER BY timestamp DESC, id DESC LIMIT ?1",
        )?;
        let mut thoughts: Vec<Thought> = stmt
            .query_map(params![limit as i64], |row| {
                let kind_str: String = row.get("kind")?;
                let memories_json: String = row.get("memories_accessed")?;
                let subjects_json: String = row.get("subjects")?;
                Ok(Thought {
                    timestamp: row.get("timestamp")?,
                    kind: ThoughtKind::parse(&kind_str).unwrap_or(ThoughtKind::Observation),
                    content: row.get("content")?,
                    memories_accessed: serde_json::from_str(&memories_json).unwrap_or_default(),
                    subjects: serde_json::from_str(&subjects_json).unwrap_or_default(),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        thoughts.reverse();
        Ok(thoughts)
    }

    // Identities, profiles, persons

    async fn add_identity(&self, identity: &Identity) -> anyhow::Result<IdentityId> {
        let conn = self.lock()?;
        let metadata_json = identity
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        conn.execute(
            "INSERT INTO identities (id, gateway_id, external_id, display_name, metadata_json, created_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(gateway_id, external_id) DO UPDATE SET
                display_name = COALESCE(excluded.display_name, identities.display_name),
                metadata_json = COALESCE(excluded.metadata_json, identities.metadata_json),
                last_seen_at = excluded.last_seen_at",
            params![
                identity.id.0,
                identity.gateway_id,
                identity.external_id,
                identity.display_name,
                metadata_json,
                identity.created_at,
                identity.last_seen_at,
            ],
        )?;
        let id = conn.query_row(
            "SELECT id FROM identities WHERE gateway_id = ?1 AND external_id = ?2",
            params![identity.gateway_id, identity.external_id],
            |row| row.get::<_, String>(0),
        )?;
        Ok(IdentityId(id))
    }

    async fn get_identity(&self, id: &IdentityId) -> anyhow::Result<Option<Identity>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT id, gateway_id, external_id, display_name, metadata_json, created_at, last_seen_at
             FROM identities WHERE id = ?1",
            params![id.0],
            read_identity,
        ) {
            Ok(identity) => Ok(Some(identity)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn resolve_identity(
        &self,
        gateway_id: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<ResolvedActorIdentity>> {
        let conn = self.lock()?;
        let identity = match conn.query_row(
            "SELECT id, gateway_id, external_id, display_name, metadata_json, created_at, last_seen_at
             FROM identities WHERE gateway_id = ?1 AND external_id = ?2",
            params![gateway_id, external_id],
            read_identity,
        ) {
            Ok(identity) => identity,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let (profile, _profile_link) = conn.query_row(
            "SELECT p.id, p.display_name, p.summary, p.comm_style, p.first_seen, p.last_seen, p.created_at, p.updated_at,
                    l.profile_id, l.identity_id, l.status, l.confidence, l.evidence_json, l.created_at, l.removed_at
             FROM profile_identities l
             JOIN profiles p ON p.id = l.profile_id
             WHERE l.identity_id = ?1 AND l.status = 'active'
             ORDER BY l.confidence DESC, l.created_at DESC
             LIMIT 1",
            params![identity.id.0],
            |row| Ok((read_profile(row)?, read_profile_identity_link(row)?)),
        )?;

        let person_link = conn
            .query_row(
                "SELECT p.id, p.display_name, p.summary, p.comm_style, p.created_at, p.updated_at,
                        l.person_id, l.profile_id, l.status, l.confidence, l.evidence_json, l.created_at, l.updated_at, l.detached_at
                 FROM person_profiles l
                 JOIN persons p ON p.id = l.person_id
                 WHERE l.profile_id = ?1 AND l.status IN ('verified', 'likely')
                 ORDER BY CASE l.status WHEN 'verified' THEN 0 ELSE 1 END, l.confidence DESC, l.updated_at DESC
                 LIMIT 1",
                params![profile.id.0],
                |row| Ok((read_person(row)?, read_person_profile_link(row)?)),
            )
            .optional()?;

        Ok(Some(ResolvedActorIdentity {
            identity,
            profile,
            person: person_link.as_ref().map(|(person, _)| person.clone()),
            profile_person_link: person_link.map(|(_, link)| link),
        }))
    }

    async fn touch_identity(&self, id: &IdentityId) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE identities SET last_seen_at = unixepoch() WHERE id = ?1",
            params![id.0],
        )?;
        Ok(())
    }

    async fn add_profile(&self, profile: &Profile) -> anyhow::Result<ProfileId> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO profiles (id, display_name, summary, comm_style, first_seen, last_seen, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                profile.id.0,
                profile.display_name,
                profile.summary,
                profile.comm_style,
                profile.first_seen,
                profile.last_seen,
                profile.created_at,
                profile.updated_at,
            ],
        )?;
        Ok(profile.id.clone())
    }

    async fn get_profile(&self, id: &ProfileId) -> anyhow::Result<Option<Profile>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT id, display_name, summary, comm_style, first_seen, last_seen, created_at, updated_at
             FROM profiles WHERE id = ?1",
            params![id.0],
            read_profile,
        ) {
            Ok(profile) => Ok(Some(profile)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn update_profile(
        &self,
        id: &ProfileId,
        display_name: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        if let Some(display_name) = display_name {
            conn.execute(
                "UPDATE profiles SET display_name = ?1, updated_at = unixepoch() WHERE id = ?2",
                params![display_name, id.0],
            )?;
        }
        if let Some(summary) = summary {
            conn.execute(
                "UPDATE profiles SET summary = ?1, updated_at = unixepoch() WHERE id = ?2",
                params![summary, id.0],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn update_profile_comm_style(&self, id: &ProfileId, style: &str) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE profiles SET comm_style = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![style, id.0],
        )?;
        Ok(())
    }

    async fn touch_profile(&self, id: &ProfileId) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE profiles SET last_seen = unixepoch(), updated_at = unixepoch() WHERE id = ?1",
            params![id.0],
        )?;
        Ok(())
    }

    async fn get_profile_for_identity(
        &self,
        identity: &IdentityId,
    ) -> anyhow::Result<Option<(Profile, ProfileIdentityLink)>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT p.id, p.display_name, p.summary, p.comm_style, p.first_seen, p.last_seen, p.created_at, p.updated_at,
                    l.profile_id, l.identity_id, l.status, l.confidence, l.evidence_json, l.created_at, l.removed_at
             FROM profile_identities l
             JOIN profiles p ON p.id = l.profile_id
             WHERE l.identity_id = ?1 AND l.status = 'active'
             ORDER BY l.confidence DESC, l.created_at DESC
             LIMIT 1",
            params![identity.0],
            |row| Ok((read_profile(row)?, read_profile_identity_link(row)?)),
        ) {
            Ok(result) => Ok(Some(result)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn link_identity_to_profile(
        &self,
        identity: &IdentityId,
        profile: &ProfileId,
        confidence: f32,
        evidence: Option<&serde_json::Value>,
    ) -> anyhow::Result<ProfileIdentityLink> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        let evidence_json = evidence.map(serde_json::to_string).transpose()?;
        conn.execute(
            "UPDATE profile_identities
             SET status = 'removed', removed_at = unixepoch()
             WHERE identity_id = ?1 AND status = 'active' AND profile_id <> ?2",
            params![identity.0, profile.0],
        )?;
        conn.execute(
            "INSERT INTO profile_identities (profile_id, identity_id, status, confidence, evidence_json, created_at, removed_at)
             VALUES (?1, ?2, 'active', ?3, ?4, unixepoch(), NULL)
             ON CONFLICT(profile_id, identity_id) DO UPDATE SET
                status = 'active',
                confidence = excluded.confidence,
                evidence_json = excluded.evidence_json,
                removed_at = NULL",
            params![profile.0, identity.0, confidence, evidence_json],
        )?;
        let link = conn.query_row(
            "SELECT profile_id, identity_id, status, confidence, evidence_json, created_at, removed_at
             FROM profile_identities WHERE profile_id = ?1 AND identity_id = ?2",
            params![profile.0, identity.0],
            read_profile_identity_link,
        )?;
        tx.commit()?;
        Ok(link)
    }

    async fn unlink_identity_from_profile(
        &self,
        identity: &IdentityId,
        profile: &ProfileId,
        reason: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let reason_json = reason.map(serde_json::to_string).transpose()?;
        conn.execute(
            "UPDATE profile_identities
             SET status = 'removed', removed_at = unixepoch(), evidence_json = COALESCE(?3, evidence_json)
             WHERE identity_id = ?1 AND profile_id = ?2 AND status = 'active'",
            params![identity.0, profile.0, reason_json],
        )?;
        Ok(())
    }

    async fn add_person(&self, person: &Person) -> anyhow::Result<PersonId> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO persons (id, display_name, summary, comm_style, created_at, updated_at)
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
            "SELECT id, display_name, summary, comm_style, created_at, updated_at FROM persons WHERE id = ?1",
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
                "UPDATE persons SET display_name = ?1, updated_at = unixepoch() WHERE id = ?2",
                params![name, id.0],
            )?;
        }
        if let Some(summary) = summary {
            conn.execute(
                "UPDATE persons SET summary = ?1, updated_at = unixepoch() WHERE id = ?2",
                params![summary, id.0],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn update_comm_style(&self, id: &PersonId, style: &str) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE persons SET comm_style = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![style, id.0],
        )?;
        Ok(())
    }

    async fn touch_person(&self, id: &PersonId) -> anyhow::Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE persons SET updated_at = unixepoch() WHERE id = ?1",
            params![id.0],
        )?;
        Ok(())
    }

    async fn list_persons(&self) -> anyhow::Result<Vec<Person>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, display_name, summary, comm_style, created_at, updated_at FROM persons ORDER BY display_name",
        )?;
        let results = stmt
            .query_map([], read_person)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    async fn attach_profile_to_person(
        &self,
        profile: &ProfileId,
        person: &PersonId,
        status: PersonProfileStatus,
        confidence: f32,
        evidence: Option<&serde_json::Value>,
    ) -> anyhow::Result<PersonProfileLink> {
        let conn = self.lock()?;
        let tx = TxGuard::begin(&conn)?;
        let evidence_json = evidence.map(serde_json::to_string).transpose()?;
        if status.is_active_person_context() {
            conn.execute(
                "UPDATE person_profiles
                 SET status = 'detached', updated_at = unixepoch(), detached_at = unixepoch()
                 WHERE profile_id = ?1 AND status IN ('verified', 'likely') AND person_id <> ?2",
                params![profile.0, person.0],
            )?;
        }
        conn.execute(
            "INSERT INTO person_profiles (person_id, profile_id, status, confidence, evidence_json, created_at, updated_at, detached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), unixepoch(), NULL)
             ON CONFLICT(person_id, profile_id) DO UPDATE SET
                status = excluded.status,
                confidence = excluded.confidence,
                evidence_json = excluded.evidence_json,
                updated_at = unixepoch(),
                detached_at = CASE WHEN excluded.status IN ('detached', 'rejected') THEN unixepoch() ELSE NULL END",
            params![person.0, profile.0, status.as_str(), confidence, evidence_json],
        )?;
        let link = conn.query_row(
            "SELECT person_id, profile_id, status, confidence, evidence_json, created_at, updated_at, detached_at
             FROM person_profiles WHERE person_id = ?1 AND profile_id = ?2",
            params![person.0, profile.0],
            read_person_profile_link,
        )?;
        tx.commit()?;
        Ok(link)
    }

    async fn detach_profile_from_person(
        &self,
        profile: &ProfileId,
        person: &PersonId,
        reason: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        let conn = self.lock()?;
        let reason_json = reason.map(serde_json::to_string).transpose()?;
        conn.execute(
            "UPDATE person_profiles
             SET status = 'detached', evidence_json = COALESCE(?3, evidence_json),
                 updated_at = unixepoch(), detached_at = unixepoch()
             WHERE profile_id = ?1 AND person_id = ?2 AND status <> 'detached'",
            params![profile.0, person.0, reason_json],
        )?;
        Ok(())
    }

    async fn get_person_for_profile(
        &self,
        profile: &ProfileId,
    ) -> anyhow::Result<Option<(Person, PersonProfileLink)>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT p.id, p.display_name, p.summary, p.comm_style, p.created_at, p.updated_at,
                    l.person_id, l.profile_id, l.status, l.confidence, l.evidence_json, l.created_at, l.updated_at, l.detached_at
             FROM person_profiles l
             JOIN persons p ON p.id = l.person_id
             WHERE l.profile_id = ?1 AND l.status IN ('verified', 'likely')
             ORDER BY CASE l.status WHEN 'verified' THEN 0 ELSE 1 END, l.confidence DESC, l.updated_at DESC
             LIMIT 1",
            params![profile.0],
            |row| Ok((read_person(row)?, read_person_profile_link(row)?)),
        ) {
            Ok(result) => Ok(Some(result)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn get_profiles_for_person(
        &self,
        person: &PersonId,
    ) -> anyhow::Result<Vec<(Profile, PersonProfileLink)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT p.id, p.display_name, p.summary, p.comm_style, p.first_seen, p.last_seen, p.created_at, p.updated_at,
                    l.person_id, l.profile_id, l.status, l.confidence, l.evidence_json, l.created_at, l.updated_at, l.detached_at
             FROM person_profiles l
             JOIN profiles p ON p.id = l.profile_id
             WHERE l.person_id = ?1
             ORDER BY l.status, l.confidence DESC, l.updated_at DESC",
        )?;
        let results = stmt
            .query_map(params![person.0], |row| {
                Ok((read_profile(row)?, read_person_profile_link(row)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    async fn get_identities_for_person(&self, person: &PersonId) -> anyhow::Result<Vec<Identity>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT i.id, i.gateway_id, i.external_id, i.display_name, i.metadata_json, i.created_at, i.last_seen_at
             FROM identities i
             JOIN profile_identities pi ON pi.identity_id = i.id AND pi.status = 'active'
             JOIN person_profiles pp ON pp.profile_id = pi.profile_id AND pp.status IN ('verified', 'likely')
             WHERE pp.person_id = ?1
             ORDER BY i.gateway_id, i.external_id",
        )?;
        let results = stmt
            .query_map(params![person.0], read_identity)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
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
