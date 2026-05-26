use super::{
    ActorSnapshot, ConversationSummary, Memory, MemoryUpdate, RecallQuery, StoredMessage, Thought,
};
use crate::identity::{
    ClaimStatus, Group, Identity, IdentityClaim, Person, Relation, SocialRelation,
};
use crate::state::{Authority, BehaviorDirective};
use async_trait::async_trait;
use protocol::{ConversationId, GroupId, MemoryId, PersonId};

#[async_trait]
pub trait Store: Send + Sync {
    // Snapshots
    async fn save_snapshot(&self, snapshot: &ActorSnapshot) -> anyhow::Result<()>;
    async fn load_latest_snapshot(&self) -> anyhow::Result<Option<ActorSnapshot>>;

    // Memories
    async fn store_memory(&self, memory: &Memory) -> anyhow::Result<MemoryId>;
    async fn get_memory(&self, id: &MemoryId) -> anyhow::Result<Option<Memory>>;
    async fn update_memory(&self, id: &MemoryId, update: &MemoryUpdate) -> anyhow::Result<()>;
    async fn recall(&self, query: &RecallQuery) -> anyhow::Result<Vec<Memory>>;
    async fn forget(&self, id: &MemoryId) -> anyhow::Result<bool>;

    // Conversations
    async fn append_message(
        &self,
        conv: &ConversationId,
        gateway_id: Option<&str>,
        group: Option<&GroupId>,
        msg: &StoredMessage,
    ) -> anyhow::Result<()>;
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
    ) -> anyhow::Result<()>;

    // Thoughts
    async fn log_thought(&self, thought: &Thought) -> anyhow::Result<()>;
    async fn recent_thoughts(&self, limit: usize) -> anyhow::Result<Vec<Thought>>;

    // People
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
    async fn list_people(&self) -> anyhow::Result<Vec<Person>>;

    // Identities
    async fn add_identity(&self, person: &PersonId, identity: &Identity) -> anyhow::Result<()>;
    async fn resolve_identity(
        &self,
        gateway_id: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<Person>>;
    async fn get_identities(&self, person: &PersonId) -> anyhow::Result<Vec<Identity>>;
    async fn merge_people(&self, keep: &PersonId, merge: &PersonId) -> anyhow::Result<()>;

    // Identity claims
    async fn create_claim(&self, claim: &IdentityClaim) -> anyhow::Result<()>;
    async fn get_pending_claims(&self) -> anyhow::Result<Vec<IdentityClaim>>;
    async fn resolve_claim(&self, claim_id: &str, status: &ClaimStatus) -> anyhow::Result<()>;

    // Social graph
    async fn add_relation(
        &self,
        a: &PersonId,
        b: &PersonId,
        relation: &Relation,
    ) -> anyhow::Result<()>;
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
        authority: &Authority,
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
}
