use super::{
    ActionMessageRecord, ActionPromptSnapshotRecord, ActionRunRecord, ActionTranscriptRecord,
    ActionTurnRecord, ActorSnapshot, ConversationSummary, DisplayNameObservation,
    EventInboxDebugRecord, EventInboxRecord, IdentityDisclosureAudit, IntentRecord,
    IntentUpdateRecord, Memory, MemoryMutationRecord, MemorySubjectDebugRecord, MemorySubjectType,
    MemoryUpdate, OutboundDeliveryRecord, RecallQuery, ReviewJobRecord, ReviewOutputAudit,
    StateJournalRecord, StoredMessage, Thought, ToolCallRecord,
};
use crate::identity::{
    ClaimStatus, Group, Identity, IdentityClaim, Person, PersonProfileLink, PersonProfileStatus,
    Profile, ProfileIdentityLink, Relation, ResolvedActorIdentity, SocialRelation,
};
use crate::state::{BehaviorDirective, RelationshipStanding};
use async_trait::async_trait;
use protocol::{ConversationId, GroupId, IdentityId, MemoryId, PersonId, ProfileId};

#[async_trait]
pub trait Store: Send + Sync {
    // Snapshots
    async fn save_snapshot(&self, snapshot: &ActorSnapshot) -> anyhow::Result<()>;
    async fn load_latest_snapshot(&self) -> anyhow::Result<Option<ActorSnapshot>>;
    async fn append_state_journal(
        &self,
        kind: &str,
        payload: &serde_json::Value,
        created_at: i64,
    ) -> anyhow::Result<i64>;
    async fn state_journal_after(
        &self,
        after_id: Option<i64>,
        limit: usize,
    ) -> anyhow::Result<Vec<StateJournalRecord>>;

    // Memories
    async fn store_memory(&self, memory: &Memory) -> anyhow::Result<MemoryId>;
    async fn get_memory(&self, id: &MemoryId) -> anyhow::Result<Option<Memory>>;
    async fn update_memory(&self, id: &MemoryId, update: &MemoryUpdate) -> anyhow::Result<()>;
    async fn recall(&self, query: &RecallQuery) -> anyhow::Result<Vec<Memory>>;
    async fn memories_for_subject(
        &self,
        subject_type: MemorySubjectType,
        subject_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Memory>>;
    async fn forget(&self, id: &MemoryId) -> anyhow::Result<bool>;
    async fn forget_with_reason(&self, id: &MemoryId, reason: Option<&str>)
    -> anyhow::Result<bool>;
    async fn memory_mutations_for_memory(
        &self,
        id: &MemoryId,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryMutationRecord>>;
    async fn prune_stale_memories(
        &self,
        now: i64,
        older_than: i64,
        max_importance: f32,
        max_confidence: f32,
        max_sensitivity: f32,
        limit: usize,
    ) -> anyhow::Result<usize>;

    // Action transcripts
    async fn start_action_run(&self, run: &ActionRunRecord) -> anyhow::Result<()>;
    async fn get_action_run(&self, action_id: &str) -> anyhow::Result<Option<ActionRunRecord>>;
    async fn finish_action_run(
        &self,
        action_id: &str,
        ended_at: i64,
        status: &str,
        responded: bool,
        attempts: u32,
        memories_formed: Vec<MemoryId>,
        recalled_memory_ids: Vec<MemoryId>,
    ) -> anyhow::Result<()>;
    async fn append_action_turn(&self, turn: &ActionTurnRecord) -> anyhow::Result<()>;
    async fn record_prompt_snapshot(
        &self,
        snapshot: &ActionPromptSnapshotRecord,
    ) -> anyhow::Result<()>;
    async fn append_tool_call(&self, call: &ToolCallRecord) -> anyhow::Result<()>;
    async fn append_action_message(&self, message: &ActionMessageRecord) -> anyhow::Result<()>;
    async fn append_outbound_delivery(
        &self,
        delivery: &OutboundDeliveryRecord,
    ) -> anyhow::Result<()>;
    async fn action_transcript(&self, action_id: &str) -> anyhow::Result<ActionTranscriptRecord>;
    async fn outbound_deliveries_for_action(
        &self,
        action_id: &str,
    ) -> anyhow::Result<Vec<OutboundDeliveryRecord>>;
    async fn mark_review_scheduled(
        &self,
        action_id: &str,
        review_action_id: &str,
        scheduled_at: i64,
    ) -> anyhow::Result<bool>;
    async fn action_review_scheduled(&self, action_id: &str) -> anyhow::Result<bool>;
    async fn record_review_output(&self, output: &ReviewOutputAudit) -> anyhow::Result<()>;
    async fn review_outputs_for_action(
        &self,
        review_action_id: &str,
    ) -> anyhow::Result<Vec<ReviewOutputAudit>>;
    async fn review_outputs_for_source_action(
        &self,
        source_action_id: &str,
    ) -> anyhow::Result<Vec<ReviewOutputAudit>>;

    // Intents
    async fn create_intent(&self, intent: &IntentRecord) -> anyhow::Result<()>;
    async fn get_intent(&self, id: &str) -> anyhow::Result<Option<IntentRecord>>;
    async fn update_intent(&self, id: &str, update: &IntentUpdateRecord) -> anyhow::Result<bool>;
    async fn cancel_intent(&self, id: &str, updated_at: i64) -> anyhow::Result<bool>;
    async fn complete_intent(&self, id: &str, updated_at: i64) -> anyhow::Result<bool>;
    async fn active_intents_for_context(
        &self,
        person: Option<&PersonId>,
        profile: Option<&ProfileId>,
        conversation: Option<&ConversationId>,
        now: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<IntentRecord>>;
    async fn due_intents(&self, now: i64, limit: usize) -> anyhow::Result<Vec<IntentRecord>>;
    async fn mark_intent_fired(&self, id: &str, fired_at: i64) -> anyhow::Result<bool>;

    // Event inbox
    async fn enqueue_event(&self, event: &EventInboxRecord) -> anyhow::Result<()>;
    async fn pending_events_by_kind(
        &self,
        kind: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<EventInboxRecord>>;
    async fn due_events(&self, now: i64, limit: usize) -> anyhow::Result<Vec<EventInboxRecord>>;
    async fn mark_event_fired(&self, id: &str, fired_at: i64) -> anyhow::Result<bool>;
    async fn mark_event_failed(
        &self,
        id: &str,
        failed_at: i64,
        error: Option<&str>,
    ) -> anyhow::Result<bool>;

    // Conversations
    async fn append_message(
        &self,
        conv: &ConversationId,
        gateway_id: Option<&str>,
        group: Option<&GroupId>,
        msg: &StoredMessage,
    ) -> anyhow::Result<()>;
    async fn update_message_content_by_source(
        &self,
        conv: &ConversationId,
        gateway_id: &str,
        source_message_id: &str,
        content: &str,
        edited_at: i64,
    ) -> anyhow::Result<bool>;
    async fn mark_message_deleted_by_source(
        &self,
        conv: &ConversationId,
        gateway_id: &str,
        source_message_id: &str,
        deleted_at: i64,
    ) -> anyhow::Result<bool>;
    async fn get_messages(
        &self,
        conv: &ConversationId,
        limit: usize,
        before: Option<i64>,
    ) -> anyhow::Result<Vec<StoredMessage>>;
    async fn list_conversations(&self) -> anyhow::Result<Vec<ConversationSummary>>;
    async fn update_conversation_summary(
        &self,
        conv: &ConversationId,
        summary: &str,
        covered_message_ids: &[String],
    ) -> anyhow::Result<()>;

    // Thoughts
    async fn log_thought(&self, thought: &Thought) -> anyhow::Result<()>;
    async fn recent_thoughts(&self, limit: usize) -> anyhow::Result<Vec<Thought>>;
    async fn recent_thoughts_for_subject(
        &self,
        subject_type: MemorySubjectType,
        subject_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Thought>>;
    async fn thoughts_for_action(
        &self,
        action_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Thought>>;
    async fn prune_stale_thoughts(
        &self,
        older_than: i64,
        max_importance: f32,
        max_confidence: f32,
        limit: usize,
    ) -> anyhow::Result<usize>;

    // Identities, profiles, persons
    async fn add_identity(&self, identity: &Identity) -> anyhow::Result<IdentityId>;
    async fn get_identity(&self, id: &IdentityId) -> anyhow::Result<Option<Identity>>;
    async fn resolve_identity(
        &self,
        gateway_id: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<ResolvedActorIdentity>>;
    async fn touch_identity(&self, id: &IdentityId) -> anyhow::Result<()>;
    async fn update_identity_display_name(
        &self,
        id: &IdentityId,
        display_name: &str,
    ) -> anyhow::Result<()>;
    async fn record_display_name_observation(
        &self,
        observation: &DisplayNameObservation,
    ) -> anyhow::Result<()>;
    async fn display_name_observations(
        &self,
        identity: &IdentityId,
        limit: usize,
    ) -> anyhow::Result<Vec<DisplayNameObservation>>;

    async fn add_profile(&self, profile: &Profile) -> anyhow::Result<ProfileId>;
    async fn get_profile(&self, id: &ProfileId) -> anyhow::Result<Option<Profile>>;
    async fn list_profiles(&self) -> anyhow::Result<Vec<Profile>>;
    async fn update_profile(
        &self,
        id: &ProfileId,
        display_name: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<()>;
    async fn update_profile_comm_style(&self, id: &ProfileId, style: &str) -> anyhow::Result<()>;
    async fn touch_profile(&self, id: &ProfileId) -> anyhow::Result<()>;
    async fn get_profile_for_identity(
        &self,
        identity: &IdentityId,
    ) -> anyhow::Result<Option<(Profile, ProfileIdentityLink)>>;
    async fn link_identity_to_profile(
        &self,
        identity: &IdentityId,
        profile: &ProfileId,
        confidence: f32,
        evidence: Option<&serde_json::Value>,
    ) -> anyhow::Result<ProfileIdentityLink>;
    async fn unlink_identity_from_profile(
        &self,
        identity: &IdentityId,
        profile: &ProfileId,
        reason: Option<&serde_json::Value>,
    ) -> anyhow::Result<()>;

    async fn add_person(&self, person: &Person) -> anyhow::Result<PersonId>;
    async fn get_person(&self, id: &PersonId) -> anyhow::Result<Option<Person>>;
    async fn update_person(
        &self,
        id: &PersonId,
        name: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<()>;
    async fn update_comm_style(&self, id: &PersonId, style: &str) -> anyhow::Result<()>;
    async fn touch_person(&self, id: &PersonId) -> anyhow::Result<()>;
    async fn list_persons(&self) -> anyhow::Result<Vec<Person>>;
    async fn attach_profile_to_person(
        &self,
        profile: &ProfileId,
        person: &PersonId,
        status: PersonProfileStatus,
        confidence: f32,
        evidence: Option<&serde_json::Value>,
    ) -> anyhow::Result<PersonProfileLink>;
    async fn detach_profile_from_person(
        &self,
        profile: &ProfileId,
        person: &PersonId,
        reason: Option<&serde_json::Value>,
    ) -> anyhow::Result<()>;
    async fn get_person_for_profile(
        &self,
        profile: &ProfileId,
    ) -> anyhow::Result<Option<(Person, PersonProfileLink)>>;
    async fn get_profiles_for_person(
        &self,
        person: &PersonId,
    ) -> anyhow::Result<Vec<(Profile, PersonProfileLink)>>;
    async fn get_identities_for_person(&self, person: &PersonId) -> anyhow::Result<Vec<Identity>>;
    async fn merge_person_context(&self, from: &PersonId, into: &PersonId) -> anyhow::Result<()>;
    async fn record_identity_disclosure(
        &self,
        audit: &IdentityDisclosureAudit,
    ) -> anyhow::Result<()>;
    async fn identity_disclosures_for_person(
        &self,
        person: &PersonId,
        limit: usize,
    ) -> anyhow::Result<Vec<IdentityDisclosureAudit>>;

    // Identity claims
    async fn create_claim(&self, claim: &IdentityClaim) -> anyhow::Result<()>;
    async fn get_pending_claims(&self) -> anyhow::Result<Vec<IdentityClaim>>;
    async fn get_recent_claims(
        &self,
        claimant: Option<&PersonId>,
        claimed_person: Option<&PersonId>,
        since: i64,
    ) -> anyhow::Result<Vec<IdentityClaim>>;
    async fn resolve_claim(&self, claim_id: &str, status: &ClaimStatus) -> anyhow::Result<()>;

    // Social graph
    async fn add_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()>;
    async fn upsert_relation(&self, relation: &SocialRelation) -> anyhow::Result<()>;
    async fn get_relations(&self, person: &PersonId) -> anyhow::Result<Vec<SocialRelation>>;
    async fn remove_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()>;

    // Groups
    async fn add_group(&self, group: &Group) -> anyhow::Result<GroupId>;
    async fn get_group(&self, id: &GroupId) -> anyhow::Result<Option<Group>>;
    async fn add_group_member(&self, group: &GroupId, person: &PersonId) -> anyhow::Result<()>;
    async fn remove_group_member(&self, group: &GroupId, person: &PersonId) -> anyhow::Result<()>;

    // Behavior directives
    async fn add_directive(&self, directive: &BehaviorDirective) -> anyhow::Result<()>;
    async fn get_directives_for_context(
        &self,
        person: &PersonId,
        relationship_standing: &RelationshipStanding,
        group: Option<&GroupId>,
    ) -> anyhow::Result<Vec<BehaviorDirective>>;
    async fn update_directive(
        &self,
        id: &str,
        directive: Option<&str>,
        active: Option<bool>,
        priority: Option<i32>,
        expires_at: Option<Option<i64>>,
    ) -> anyhow::Result<()>;
    async fn remove_directive(&self, id: &str) -> anyhow::Result<bool>;
    async fn list_directives(&self) -> anyhow::Result<Vec<BehaviorDirective>>;

    // Debug views
    async fn debug_recent_memories(&self, limit: usize) -> anyhow::Result<Vec<Memory>>;
    async fn debug_memory_subjects(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<MemorySubjectDebugRecord>>;
    async fn debug_profile_identity_links(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<ProfileIdentityLink>>;
    async fn debug_person_profile_links(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PersonProfileLink>>;
    async fn debug_groups(&self, limit: usize) -> anyhow::Result<Vec<Group>>;
    async fn debug_active_intents(&self, limit: usize) -> anyhow::Result<Vec<IntentRecord>>;
    async fn debug_recent_review_outputs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<ReviewOutputAudit>>;
    async fn debug_recent_review_jobs(&self, limit: usize) -> anyhow::Result<Vec<ReviewJobRecord>>;
    async fn debug_recent_action_runs(&self, limit: usize) -> anyhow::Result<Vec<ActionRunRecord>>;
    async fn debug_recent_memory_mutations(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryMutationRecord>>;
    async fn debug_recent_failed_events(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<EventInboxDebugRecord>>;
}
