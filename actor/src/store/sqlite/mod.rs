use super::{
    ActionMessageRecord, ActionPromptSnapshotRecord, ActionRunRecord, ActionTranscriptRecord,
    ActionTurnRecord, ActorSnapshot, ConversationSummary, DisplayNameObservation,
    EventInboxDebugRecord, EventInboxRecord, IdentityDisclosureAudit, IntentRecord,
    IntentUpdateRecord, Memory, MemoryMutationRecord, MemorySubjectDebugRecord, MemorySubjectType,
    MemoryUpdate, OutboundDeliveryRecord, RecallQuery, ReviewJobRecord, ReviewOutputAudit,
    StateJournalRecord, Store, StoredMessage, Thought, ToolCallRecord,
};
use crate::identity::{
    ClaimStatus, Group, Identity, IdentityClaim, Person, PersonProfileLink, PersonProfileStatus,
    Profile, ProfileIdentityLink, Relation, ResolvedActorIdentity, SocialRelation,
};
#[cfg(test)]
use crate::identity::{GroupContext, RelationSource, RelationStatus};
use crate::state::{BehaviorDirective, RelationshipStanding};
#[cfg(test)]
use crate::store::{MemorySubject, ThoughtKind};
use protocol::{ConversationId, GroupId, IdentityId, MemoryId, PersonId, ProfileId};
use rusqlite::Connection;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
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

impl SqliteStore {
    pub fn open(config: SqliteConfig) -> anyhow::Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open(&config.path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        init_schema(&conn, config.embedding_dimensions)?;
        memory::seed_actor_identity_memories(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory(embedding_dimensions: usize) -> anyhow::Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        init_schema(&conn, embedding_dimensions)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    #[cfg(test)]
    fn lock(&self) -> anyhow::Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("lock poisoned"))
    }

    async fn with_conn_blocking<T, F>(&self, op: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| anyhow::anyhow!("lock poisoned"))?;
            op(&conn)
        })
        .await?
    }
}

mod action_log;
mod conversation;
mod debug;
mod directive;
mod event_inbox;
mod group;
mod identity;
mod intent;
mod memory;
mod migrations;
mod person;
mod rows;
mod schema;
mod snapshot;
mod social_graph;
mod support;
mod thought;

#[cfg(test)]
mod tests;

use schema::init_schema;
use support::register_sqlite_vec;
#[cfg(test)]
use support::sqlite_query_is_slow;
#[async_trait::async_trait]
impl Store for SqliteStore {
    async fn save_snapshot(&self, snapshot: &ActorSnapshot) -> anyhow::Result<()> {
        let snapshot = snapshot.clone();
        self.with_conn_blocking(move |conn| snapshot::save_snapshot(conn, &snapshot))
            .await
    }

    async fn load_latest_snapshot(&self) -> anyhow::Result<Option<ActorSnapshot>> {
        self.with_conn_blocking(snapshot::load_latest_snapshot)
            .await
    }

    async fn append_state_journal(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        created_at: i64,
    ) -> anyhow::Result<i64> {
        let kind = kind.to_string();
        let payload = payload.clone();
        self.with_conn_blocking(move |conn| {
            snapshot::append_state_journal(conn, &kind, &payload, created_at)
        })
        .await
    }

    async fn state_journal_after(
        &self,
        after_id: Option<i64>,
        limit: usize,
    ) -> anyhow::Result<Vec<StateJournalRecord>> {
        self.with_conn_blocking(move |conn| snapshot::state_journal_after(conn, after_id, limit))
            .await
    }

    async fn store_memory(&self, memory: &Memory) -> anyhow::Result<MemoryId> {
        let memory = memory.clone();
        self.with_conn_blocking(move |conn| memory::store_memory(conn, &memory))
            .await
    }

    async fn get_memory(&self, id: &MemoryId) -> anyhow::Result<Option<Memory>> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| memory::get_memory(conn, &id))
            .await
    }

    async fn update_memory(&self, id: &MemoryId, update: &MemoryUpdate) -> anyhow::Result<()> {
        let id = id.clone();
        let update = update.clone();
        self.with_conn_blocking(move |conn| memory::update_memory(conn, &id, &update))
            .await
    }

    async fn recall(&self, query: &RecallQuery) -> anyhow::Result<Vec<Memory>> {
        let query = query.clone();
        self.with_conn_blocking(move |conn| memory::recall(conn, &query))
            .await
    }

    async fn memories_for_subject(
        &self,
        subject_type: MemorySubjectType,
        subject_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Memory>> {
        let subject_id = subject_id.to_string();
        self.with_conn_blocking(move |conn| {
            memory::memories_for_subject(conn, subject_type, &subject_id, limit)
        })
        .await
    }

    async fn forget(&self, id: &MemoryId) -> anyhow::Result<bool> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| memory::forget(conn, &id, None))
            .await
    }

    async fn forget_with_reason(
        &self,
        id: &MemoryId,
        reason: Option<&str>,
    ) -> anyhow::Result<bool> {
        let id = id.clone();
        let reason = reason.map(str::to_string);
        self.with_conn_blocking(move |conn| memory::forget(conn, &id, reason.as_deref()))
            .await
    }

    async fn memory_mutations_for_memory(
        &self,
        id: &MemoryId,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryMutationRecord>> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| memory::memory_mutations_for_memory(conn, &id, limit))
            .await
    }

    async fn prune_stale_memories(
        &self,
        now: i64,
        older_than: i64,
        max_importance: f32,
        max_confidence: f32,
        max_sensitivity: f32,
        limit: usize,
    ) -> anyhow::Result<usize> {
        self.with_conn_blocking(move |conn| {
            memory::prune_stale_memories(
                conn,
                now,
                older_than,
                max_importance,
                max_confidence,
                max_sensitivity,
                limit,
            )
        })
        .await
    }

    async fn start_action_run(&self, run: &ActionRunRecord) -> anyhow::Result<()> {
        let run = run.clone();
        self.with_conn_blocking(move |conn| action_log::start_action_run(conn, &run))
            .await
    }

    async fn get_action_run(&self, action_id: &str) -> anyhow::Result<Option<ActionRunRecord>> {
        let action_id = action_id.to_string();
        self.with_conn_blocking(move |conn| action_log::get_action_run(conn, &action_id))
            .await
    }

    async fn finish_action_run(
        &self,
        action_id: &str,
        ended_at: i64,
        status: &str,
        responded: bool,
        attempts: u32,
        memories_formed: Vec<MemoryId>,
        recalled_memory_ids: Vec<MemoryId>,
    ) -> anyhow::Result<()> {
        let action_id = action_id.to_string();
        let status = status.to_string();
        self.with_conn_blocking(move |conn| {
            action_log::finish_action_run(
                conn,
                &action_id,
                ended_at,
                &status,
                responded,
                attempts,
                &memories_formed,
                &recalled_memory_ids,
            )
        })
        .await
    }

    async fn append_action_turn(&self, turn: &ActionTurnRecord) -> anyhow::Result<()> {
        let turn = turn.clone();
        self.with_conn_blocking(move |conn| action_log::append_action_turn(conn, &turn))
            .await
    }

    async fn record_prompt_snapshot(
        &self,
        snapshot: &ActionPromptSnapshotRecord,
    ) -> anyhow::Result<()> {
        let snapshot = snapshot.clone();
        self.with_conn_blocking(move |conn| action_log::record_prompt_snapshot(conn, &snapshot))
            .await
    }

    async fn append_tool_call(&self, call: &ToolCallRecord) -> anyhow::Result<()> {
        let call = call.clone();
        self.with_conn_blocking(move |conn| action_log::append_tool_call(conn, &call))
            .await
    }

    async fn append_action_message(&self, message: &ActionMessageRecord) -> anyhow::Result<()> {
        let message = message.clone();
        self.with_conn_blocking(move |conn| action_log::append_action_message(conn, &message))
            .await
    }

    async fn append_outbound_delivery(
        &self,
        delivery: &OutboundDeliveryRecord,
    ) -> anyhow::Result<()> {
        let delivery = delivery.clone();
        self.with_conn_blocking(move |conn| action_log::append_outbound_delivery(conn, &delivery))
            .await
    }

    async fn action_transcript(&self, action_id: &str) -> anyhow::Result<ActionTranscriptRecord> {
        let action_id = action_id.to_string();
        self.with_conn_blocking(move |conn| action_log::action_transcript(conn, &action_id))
            .await
    }

    async fn outbound_deliveries_for_action(
        &self,
        action_id: &str,
    ) -> anyhow::Result<Vec<OutboundDeliveryRecord>> {
        let action_id = action_id.to_string();
        self.with_conn_blocking(move |conn| {
            action_log::outbound_deliveries_for_action(conn, &action_id)
        })
        .await
    }

    async fn mark_review_scheduled(
        &self,
        action_id: &str,
        review_action_id: &str,
        scheduled_at: i64,
    ) -> anyhow::Result<bool> {
        let action_id = action_id.to_string();
        let review_action_id = review_action_id.to_string();
        self.with_conn_blocking(move |conn| {
            action_log::mark_review_scheduled(conn, &action_id, &review_action_id, scheduled_at)
        })
        .await
    }

    async fn action_review_scheduled(&self, action_id: &str) -> anyhow::Result<bool> {
        let action_id = action_id.to_string();
        self.with_conn_blocking(move |conn| action_log::action_review_scheduled(conn, &action_id))
            .await
    }

    async fn record_review_output(&self, output: &ReviewOutputAudit) -> anyhow::Result<()> {
        let output = output.clone();
        self.with_conn_blocking(move |conn| action_log::record_review_output(conn, &output))
            .await
    }

    async fn review_outputs_for_action(
        &self,
        review_action_id: &str,
    ) -> anyhow::Result<Vec<ReviewOutputAudit>> {
        let review_action_id = review_action_id.to_string();
        self.with_conn_blocking(move |conn| {
            action_log::review_outputs_for_action(conn, &review_action_id)
        })
        .await
    }

    async fn review_outputs_for_source_action(
        &self,
        source_action_id: &str,
    ) -> anyhow::Result<Vec<ReviewOutputAudit>> {
        let source_action_id = source_action_id.to_string();
        self.with_conn_blocking(move |conn| {
            action_log::review_outputs_for_source_action(conn, &source_action_id)
        })
        .await
    }

    async fn create_intent(&self, intent: &IntentRecord) -> anyhow::Result<()> {
        let intent = intent.clone();
        self.with_conn_blocking(move |conn| intent::create_intent(conn, &intent))
            .await
    }

    async fn get_intent(&self, id: &str) -> anyhow::Result<Option<IntentRecord>> {
        let id = id.to_string();
        self.with_conn_blocking(move |conn| intent::get_intent(conn, &id))
            .await
    }

    async fn update_intent(&self, id: &str, update: &IntentUpdateRecord) -> anyhow::Result<bool> {
        let id = id.to_string();
        let update = update.clone();
        self.with_conn_blocking(move |conn| intent::update_intent(conn, &id, &update))
            .await
    }

    async fn cancel_intent(&self, id: &str, updated_at: i64) -> anyhow::Result<bool> {
        let id = id.to_string();
        self.with_conn_blocking(move |conn| intent::cancel_intent(conn, &id, updated_at))
            .await
    }

    async fn complete_intent(&self, id: &str, updated_at: i64) -> anyhow::Result<bool> {
        let id = id.to_string();
        self.with_conn_blocking(move |conn| intent::complete_intent(conn, &id, updated_at))
            .await
    }

    async fn active_intents_for_context(
        &self,
        person: Option<&PersonId>,
        profile: Option<&ProfileId>,
        conversation: Option<&ConversationId>,
        now: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<IntentRecord>> {
        let person = person.cloned();
        let profile = profile.cloned();
        let conversation = conversation.cloned();
        self.with_conn_blocking(move |conn| {
            intent::active_intents_for_context(
                conn,
                person.as_ref(),
                profile.as_ref(),
                conversation.as_ref(),
                now,
                limit,
            )
        })
        .await
    }

    async fn due_intents(&self, now: i64, limit: usize) -> anyhow::Result<Vec<IntentRecord>> {
        self.with_conn_blocking(move |conn| intent::due_intents(conn, now, limit))
            .await
    }

    async fn mark_intent_fired(&self, id: &str, fired_at: i64) -> anyhow::Result<bool> {
        let id = id.to_string();
        self.with_conn_blocking(move |conn| intent::mark_intent_fired(conn, &id, fired_at))
            .await
    }

    async fn enqueue_event(&self, event: &EventInboxRecord) -> anyhow::Result<()> {
        let event = event.clone();
        self.with_conn_blocking(move |conn| event_inbox::enqueue_event(conn, &event))
            .await
    }

    async fn pending_events_by_kind(
        &self,
        kind: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<EventInboxRecord>> {
        let kind = kind.to_string();
        self.with_conn_blocking(move |conn| event_inbox::pending_events_by_kind(conn, &kind, limit))
            .await
    }

    async fn due_events(&self, now: i64, limit: usize) -> anyhow::Result<Vec<EventInboxRecord>> {
        self.with_conn_blocking(move |conn| event_inbox::due_events(conn, now, limit))
            .await
    }

    async fn mark_event_fired(&self, id: &str, fired_at: i64) -> anyhow::Result<bool> {
        let id = id.to_string();
        self.with_conn_blocking(move |conn| event_inbox::mark_event_fired(conn, &id, fired_at))
            .await
    }

    async fn mark_event_failed(
        &self,
        id: &str,
        failed_at: i64,
        error: Option<&str>,
    ) -> anyhow::Result<bool> {
        let id = id.to_string();
        let error = error.map(str::to_string);
        self.with_conn_blocking(move |conn| {
            event_inbox::mark_event_failed(conn, &id, failed_at, error.as_deref())
        })
        .await
    }

    async fn append_message(
        &self,
        conv: &ConversationId,
        gateway_id: Option<&str>,
        group: Option<&GroupId>,
        msg: &StoredMessage,
    ) -> anyhow::Result<()> {
        let conv = conv.clone();
        let gateway_id = gateway_id.map(str::to_string);
        let group = group.cloned();
        let msg = msg.clone();
        self.with_conn_blocking(move |conn| {
            conversation::append_message(conn, &conv, gateway_id.as_deref(), group.as_ref(), &msg)
        })
        .await
    }

    async fn update_message_content_by_source(
        &self,
        conv: &ConversationId,
        gateway_id: &str,
        source_message_id: &str,
        content: &str,
        edited_at: i64,
    ) -> anyhow::Result<bool> {
        let conv = conv.clone();
        let gateway_id = gateway_id.to_string();
        let source_message_id = source_message_id.to_string();
        let content = content.to_string();
        self.with_conn_blocking(move |conn| {
            conversation::update_message_content_by_source(
                conn,
                &conv,
                &gateway_id,
                &source_message_id,
                &content,
                edited_at,
            )
        })
        .await
    }

    async fn mark_message_deleted_by_source(
        &self,
        conv: &ConversationId,
        gateway_id: &str,
        source_message_id: &str,
        deleted_at: i64,
    ) -> anyhow::Result<bool> {
        let conv = conv.clone();
        let gateway_id = gateway_id.to_string();
        let source_message_id = source_message_id.to_string();
        self.with_conn_blocking(move |conn| {
            conversation::mark_message_deleted_by_source(
                conn,
                &conv,
                &gateway_id,
                &source_message_id,
                deleted_at,
            )
        })
        .await
    }

    async fn get_messages(
        &self,
        conv: &ConversationId,
        limit: usize,
        before: Option<i64>,
    ) -> anyhow::Result<Vec<StoredMessage>> {
        let conv = conv.clone();
        self.with_conn_blocking(move |conn| conversation::get_messages(conn, &conv, limit, before))
            .await
    }

    async fn list_conversations(&self) -> anyhow::Result<Vec<ConversationSummary>> {
        self.with_conn_blocking(conversation::list_conversations)
            .await
    }

    async fn update_conversation_summary(
        &self,
        conv: &ConversationId,
        summary: &str,
        covered_message_ids: &[String],
    ) -> anyhow::Result<()> {
        let conv = conv.clone();
        let summary = summary.to_string();
        let covered_message_ids = covered_message_ids.to_vec();
        self.with_conn_blocking(move |conn| {
            conversation::update_conversation_summary(conn, &conv, &summary, &covered_message_ids)
        })
        .await
    }

    async fn log_thought(&self, thought: &Thought) -> anyhow::Result<()> {
        let thought = thought.clone();
        self.with_conn_blocking(move |conn| thought::log_thought(conn, &thought))
            .await
    }

    async fn recent_thoughts(&self, limit: usize) -> anyhow::Result<Vec<Thought>> {
        self.with_conn_blocking(move |conn| thought::recent_thoughts(conn, limit))
            .await
    }

    async fn recent_thoughts_for_subject(
        &self,
        subject_type: MemorySubjectType,
        subject_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Thought>> {
        let subject_id = subject_id.to_string();
        self.with_conn_blocking(move |conn| {
            thought::recent_thoughts_for_subject(conn, subject_type, &subject_id, limit)
        })
        .await
    }

    async fn thoughts_for_action(
        &self,
        action_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Thought>> {
        let action_id = action_id.to_string();
        self.with_conn_blocking(move |conn| thought::thoughts_for_action(conn, &action_id, limit))
            .await
    }

    async fn prune_stale_thoughts(
        &self,
        older_than: i64,
        max_importance: f32,
        max_confidence: f32,
        limit: usize,
    ) -> anyhow::Result<usize> {
        self.with_conn_blocking(move |conn| {
            thought::prune_stale_thoughts(conn, older_than, max_importance, max_confidence, limit)
        })
        .await
    }

    // Identities, profiles, persons

    async fn add_identity(&self, identity: &Identity) -> anyhow::Result<IdentityId> {
        let identity = identity.clone();
        self.with_conn_blocking(move |conn| identity::add_identity(conn, &identity))
            .await
    }

    async fn get_identity(&self, id: &IdentityId) -> anyhow::Result<Option<Identity>> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| identity::get_identity(conn, &id))
            .await
    }

    async fn resolve_identity(
        &self,
        gateway_id: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<ResolvedActorIdentity>> {
        let gateway_id = gateway_id.to_string();
        let external_id = external_id.to_string();
        self.with_conn_blocking(move |conn| {
            identity::resolve_identity(conn, &gateway_id, &external_id)
        })
        .await
    }

    async fn touch_identity(&self, id: &IdentityId) -> anyhow::Result<()> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| identity::touch_identity(conn, &id))
            .await
    }

    async fn update_identity_display_name(
        &self,
        id: &IdentityId,
        display_name: &str,
    ) -> anyhow::Result<()> {
        let id = id.clone();
        let display_name = display_name.to_string();
        self.with_conn_blocking(move |conn| {
            identity::update_identity_display_name(conn, &id, &display_name)
        })
        .await
    }

    async fn record_display_name_observation(
        &self,
        observation: &DisplayNameObservation,
    ) -> anyhow::Result<()> {
        let observation = observation.clone();
        self.with_conn_blocking(move |conn| {
            identity::record_display_name_observation(conn, &observation)
        })
        .await
    }

    async fn display_name_observations(
        &self,
        identity: &IdentityId,
        limit: usize,
    ) -> anyhow::Result<Vec<DisplayNameObservation>> {
        let identity = identity.clone();
        self.with_conn_blocking(move |conn| {
            identity::display_name_observations(conn, &identity, limit)
        })
        .await
    }

    async fn add_profile(&self, profile: &Profile) -> anyhow::Result<ProfileId> {
        let profile = profile.clone();
        self.with_conn_blocking(move |conn| person::add_profile(conn, &profile))
            .await
    }

    async fn get_profile(&self, id: &ProfileId) -> anyhow::Result<Option<Profile>> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| person::get_profile(conn, &id))
            .await
    }

    async fn list_profiles(&self) -> anyhow::Result<Vec<Profile>> {
        self.with_conn_blocking(person::list_profiles).await
    }

    async fn update_profile(
        &self,
        id: &ProfileId,
        display_name: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = id.clone();
        let display_name = display_name.map(str::to_string);
        let summary = summary.map(str::to_string);
        self.with_conn_blocking(move |conn| {
            person::update_profile(conn, &id, display_name.as_deref(), summary.as_deref())
        })
        .await
    }

    async fn update_profile_comm_style(&self, id: &ProfileId, style: &str) -> anyhow::Result<()> {
        let id = id.clone();
        let style = style.to_string();
        self.with_conn_blocking(move |conn| person::update_profile_comm_style(conn, &id, &style))
            .await
    }

    async fn touch_profile(&self, id: &ProfileId) -> anyhow::Result<()> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| person::touch_profile(conn, &id))
            .await
    }

    async fn get_profile_for_identity(
        &self,
        identity: &IdentityId,
    ) -> anyhow::Result<Option<(Profile, ProfileIdentityLink)>> {
        let identity = identity.clone();
        self.with_conn_blocking(move |conn| identity::get_profile_for_identity(conn, &identity))
            .await
    }

    async fn link_identity_to_profile(
        &self,
        identity: &IdentityId,
        profile: &ProfileId,
        confidence: f32,
        evidence: Option<&serde_json::Value>,
    ) -> anyhow::Result<ProfileIdentityLink> {
        let identity = identity.clone();
        let profile = profile.clone();
        let evidence = evidence.cloned();
        self.with_conn_blocking(move |conn| {
            identity::link_identity_to_profile(
                conn,
                &identity,
                &profile,
                confidence,
                evidence.as_ref(),
            )
        })
        .await
    }

    async fn unlink_identity_from_profile(
        &self,
        identity: &IdentityId,
        profile: &ProfileId,
        reason: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        let identity = identity.clone();
        let profile = profile.clone();
        let reason = reason.cloned();
        self.with_conn_blocking(move |conn| {
            identity::unlink_identity_from_profile(conn, &identity, &profile, reason.as_ref())
        })
        .await
    }

    async fn add_person(&self, person: &Person) -> anyhow::Result<PersonId> {
        let person = person.clone();
        self.with_conn_blocking(move |conn| person::add_person(conn, &person))
            .await
    }

    async fn get_person(&self, id: &PersonId) -> anyhow::Result<Option<Person>> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| person::get_person(conn, &id))
            .await
    }

    async fn update_person(
        &self,
        id: &PersonId,
        name: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = id.clone();
        let name = name.map(str::to_string);
        let summary = summary.map(str::to_string);
        self.with_conn_blocking(move |conn| {
            person::update_person(conn, &id, name.as_deref(), summary.as_deref())
        })
        .await
    }

    async fn update_comm_style(&self, id: &PersonId, style: &str) -> anyhow::Result<()> {
        let id = id.clone();
        let style = style.to_string();
        self.with_conn_blocking(move |conn| person::update_comm_style(conn, &id, &style))
            .await
    }

    async fn touch_person(&self, id: &PersonId) -> anyhow::Result<()> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| person::touch_person(conn, &id))
            .await
    }

    async fn list_persons(&self) -> anyhow::Result<Vec<Person>> {
        self.with_conn_blocking(person::list_persons).await
    }

    async fn attach_profile_to_person(
        &self,
        profile: &ProfileId,
        person: &PersonId,
        status: PersonProfileStatus,
        confidence: f32,
        evidence: Option<&serde_json::Value>,
    ) -> anyhow::Result<PersonProfileLink> {
        let profile = profile.clone();
        let person = person.clone();
        let evidence = evidence.cloned();
        self.with_conn_blocking(move |conn| {
            person::attach_profile_to_person(
                conn,
                &profile,
                &person,
                status,
                confidence,
                evidence.as_ref(),
            )
        })
        .await
    }

    async fn detach_profile_from_person(
        &self,
        profile: &ProfileId,
        person: &PersonId,
        reason: Option<&serde_json::Value>,
    ) -> anyhow::Result<()> {
        let profile = profile.clone();
        let person = person.clone();
        let reason = reason.cloned();
        self.with_conn_blocking(move |conn| {
            person::detach_profile_from_person(conn, &profile, &person, reason.as_ref())
        })
        .await
    }

    async fn get_person_for_profile(
        &self,
        profile: &ProfileId,
    ) -> anyhow::Result<Option<(Person, PersonProfileLink)>> {
        let profile = profile.clone();
        self.with_conn_blocking(move |conn| person::get_person_for_profile(conn, &profile))
            .await
    }

    async fn get_profiles_for_person(
        &self,
        person: &PersonId,
    ) -> anyhow::Result<Vec<(Profile, PersonProfileLink)>> {
        let person = person.clone();
        self.with_conn_blocking(move |conn| person::get_profiles_for_person(conn, &person))
            .await
    }

    async fn get_identities_for_person(&self, person: &PersonId) -> anyhow::Result<Vec<Identity>> {
        let person = person.clone();
        self.with_conn_blocking(move |conn| person::get_identities_for_person(conn, &person))
            .await
    }

    async fn record_identity_disclosure(
        &self,
        audit: &IdentityDisclosureAudit,
    ) -> anyhow::Result<()> {
        let audit = audit.clone();
        self.with_conn_blocking(move |conn| identity::record_identity_disclosure(conn, &audit))
            .await
    }

    async fn identity_disclosures_for_person(
        &self,
        person: &PersonId,
        limit: usize,
    ) -> anyhow::Result<Vec<IdentityDisclosureAudit>> {
        let person = person.clone();
        self.with_conn_blocking(move |conn| {
            identity::identity_disclosures_for_person(conn, &person, limit)
        })
        .await
    }

    async fn merge_person_context(&self, from: &PersonId, into: &PersonId) -> anyhow::Result<()> {
        let from = from.clone();
        let into = into.clone();
        self.with_conn_blocking(move |conn| person::merge_person_context(conn, &from, &into))
            .await
    }

    // Identity claims

    async fn create_claim(&self, claim: &IdentityClaim) -> anyhow::Result<()> {
        let claim = claim.clone();
        self.with_conn_blocking(move |conn| identity::create_claim(conn, &claim))
            .await
    }

    async fn get_pending_claims(&self) -> anyhow::Result<Vec<IdentityClaim>> {
        self.with_conn_blocking(identity::get_pending_claims).await
    }

    async fn get_recent_claims(
        &self,
        claimant: Option<&PersonId>,
        claimed_person: Option<&PersonId>,
        since: i64,
    ) -> anyhow::Result<Vec<IdentityClaim>> {
        let claimant = claimant.cloned();
        let claimed_person = claimed_person.cloned();
        self.with_conn_blocking(move |conn| {
            identity::get_recent_claims(conn, claimant.as_ref(), claimed_person.as_ref(), since)
        })
        .await
    }

    async fn resolve_claim(&self, claim_id: &str, status: &ClaimStatus) -> anyhow::Result<()> {
        let claim_id = claim_id.to_string();
        let status = status.clone();
        self.with_conn_blocking(move |conn| identity::resolve_claim(conn, &claim_id, &status))
            .await
    }

    // Social graph

    async fn add_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()> {
        let a = a.clone();
        let b = b.clone();
        let relation = relation.clone();
        self.with_conn_blocking(move |conn| social_graph::add_relation(conn, &a, &b, &relation))
            .await
    }

    async fn upsert_relation(&self, relation: &SocialRelation) -> anyhow::Result<()> {
        let relation = relation.clone();
        self.with_conn_blocking(move |conn| social_graph::upsert_relation(conn, &relation))
            .await
    }

    async fn get_relations(&self, person: &PersonId) -> anyhow::Result<Vec<SocialRelation>> {
        let person = person.clone();
        self.with_conn_blocking(move |conn| social_graph::get_relations(conn, &person))
            .await
    }

    async fn remove_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()> {
        let a = a.clone();
        let b = b.clone();
        let relation = relation.clone();
        self.with_conn_blocking(move |conn| social_graph::remove_relation(conn, &a, &b, &relation))
            .await
    }

    // Groups

    async fn add_group(&self, group: &Group) -> anyhow::Result<GroupId> {
        let group = group.clone();
        self.with_conn_blocking(move |conn| group::add_group(conn, &group))
            .await
    }

    async fn get_group(&self, id: &GroupId) -> anyhow::Result<Option<Group>> {
        let id = id.clone();
        self.with_conn_blocking(move |conn| group::get_group(conn, &id))
            .await
    }

    async fn add_group_member(&self, group: &GroupId, person: &PersonId) -> anyhow::Result<()> {
        let group = group.clone();
        let person = person.clone();
        self.with_conn_blocking(move |conn| group::add_group_member(conn, &group, &person))
            .await
    }

    async fn remove_group_member(&self, group: &GroupId, person: &PersonId) -> anyhow::Result<()> {
        let group = group.clone();
        let person = person.clone();
        self.with_conn_blocking(move |conn| group::remove_group_member(conn, &group, &person))
            .await
    }

    // Behavior directives

    async fn add_directive(&self, directive: &BehaviorDirective) -> anyhow::Result<()> {
        let directive = directive.clone();
        self.with_conn_blocking(move |conn| directive::add_directive(conn, &directive))
            .await
    }

    async fn get_directives_for_context(
        &self,
        person: &PersonId,
        relationship_standing: &RelationshipStanding,
        group: Option<&GroupId>,
    ) -> anyhow::Result<Vec<BehaviorDirective>> {
        let person = person.clone();
        let relationship_standing = relationship_standing.clone();
        let group = group.cloned();
        self.with_conn_blocking(move |conn| {
            directive::get_directives_for_context(
                conn,
                &person,
                &relationship_standing,
                group.as_ref(),
            )
        })
        .await
    }

    async fn update_directive(
        &self,
        id: &str,
        directive: Option<&str>,
        active: Option<bool>,
        priority: Option<i32>,
        expires_at: Option<Option<i64>>,
    ) -> anyhow::Result<()> {
        let id = id.to_string();
        let directive = directive.map(str::to_string);
        self.with_conn_blocking(move |conn| {
            directive::update_directive(
                conn,
                &id,
                directive.as_deref(),
                active,
                priority,
                expires_at,
            )
        })
        .await
    }

    async fn remove_directive(&self, id: &str) -> anyhow::Result<bool> {
        let id = id.to_string();
        self.with_conn_blocking(move |conn| directive::remove_directive(conn, &id))
            .await
    }

    async fn list_directives(&self) -> anyhow::Result<Vec<BehaviorDirective>> {
        self.with_conn_blocking(directive::list_directives).await
    }

    async fn debug_recent_memories(&self, limit: usize) -> anyhow::Result<Vec<Memory>> {
        self.with_conn_blocking(move |conn| debug::recent_memories(conn, limit))
            .await
    }

    async fn debug_memory_subjects(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<MemorySubjectDebugRecord>> {
        self.with_conn_blocking(move |conn| debug::memory_subjects(conn, limit))
            .await
    }

    async fn debug_profile_identity_links(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<ProfileIdentityLink>> {
        self.with_conn_blocking(move |conn| debug::profile_identity_links(conn, limit))
            .await
    }

    async fn debug_person_profile_links(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PersonProfileLink>> {
        self.with_conn_blocking(move |conn| debug::person_profile_links(conn, limit))
            .await
    }

    async fn debug_groups(&self, limit: usize) -> anyhow::Result<Vec<Group>> {
        self.with_conn_blocking(move |conn| group::list_groups(conn, limit))
            .await
    }

    async fn debug_active_intents(&self, limit: usize) -> anyhow::Result<Vec<IntentRecord>> {
        self.with_conn_blocking(move |conn| debug::active_intents(conn, limit))
            .await
    }

    async fn debug_recent_review_outputs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<ReviewOutputAudit>> {
        self.with_conn_blocking(move |conn| debug::recent_review_outputs(conn, limit))
            .await
    }

    async fn debug_recent_review_jobs(&self, limit: usize) -> anyhow::Result<Vec<ReviewJobRecord>> {
        self.with_conn_blocking(move |conn| debug::recent_review_jobs(conn, limit))
            .await
    }

    async fn debug_recent_action_runs(&self, limit: usize) -> anyhow::Result<Vec<ActionRunRecord>> {
        self.with_conn_blocking(move |conn| debug::recent_action_runs(conn, limit))
            .await
    }

    async fn debug_recent_memory_mutations(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryMutationRecord>> {
        self.with_conn_blocking(move |conn| debug::recent_memory_mutations(conn, limit))
            .await
    }

    async fn debug_recent_failed_events(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<EventInboxDebugRecord>> {
        self.with_conn_blocking(move |conn| debug::recent_failed_events(conn, limit))
            .await
    }
}
