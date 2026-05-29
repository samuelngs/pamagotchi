use protocol::{ChannelId, IdentityId, MemoryId, PersonId, ProfileId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityDisclosureAudit {
    pub id: String,
    pub action_id: String,
    pub requester_person: Option<PersonId>,
    pub target_person: PersonId,
    pub reason: String,
    pub allowed: bool,
    pub identity_count: u32,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayNameObservation {
    pub identity: IdentityId,
    pub profile: Option<ProfileId>,
    pub gateway_id: String,
    pub external_id: String,
    pub display_name: String,
    pub source_message_id: Option<String>,
    pub observed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdentityConflictRecord {
    pub id: String,
    pub channel: Option<ChannelId>,
    pub platform_message_id: Option<String>,
    pub primary_identity: Option<IdentityId>,
    pub reason: String,
    pub status: String,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub resolution: Value,
    pub identities: Vec<IdentityConflictIdentity>,
    pub profiles: Vec<ProfileId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityConflictIdentity {
    pub identity: IdentityId,
    pub role: String,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewOutputAudit {
    pub id: String,
    pub review_action_id: String,
    pub source_action_id: Option<String>,
    pub input: Value,
    pub result: Value,
    pub applied_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryMutationRecord {
    pub id: i64,
    pub memory: MemoryId,
    pub operation: String,
    pub reason: Option<String>,
    pub data: Value,
    pub created_at: i64,
}
