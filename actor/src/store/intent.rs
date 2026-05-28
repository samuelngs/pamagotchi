use protocol::{ConversationId, MemoryId, PersonId, ProfileId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntentRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub task: String,
    pub person: Option<PersonId>,
    pub profile: Option<ProfileId>,
    pub conversation: Option<ConversationId>,
    pub fire_at: Option<i64>,
    pub condition: Option<String>,
    pub recurrence: Option<String>,
    pub priority: u8,
    pub dedupe_key: Option<String>,
    pub source_action: Option<String>,
    pub source_memory: Option<MemoryId>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_fired_at: Option<i64>,
    #[serde(default)]
    pub chosen_person_approved: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IntentUpdateRecord {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub task: Option<String>,
    pub person: Option<PersonId>,
    pub profile: Option<ProfileId>,
    pub conversation: Option<ConversationId>,
    pub fire_at: Option<i64>,
    pub condition: Option<String>,
    pub recurrence: Option<String>,
    pub priority: Option<u8>,
    pub dedupe_key: Option<String>,
    pub source_memory: Option<MemoryId>,
    pub chosen_person_approved: Option<bool>,
    pub updated_at: i64,
}
