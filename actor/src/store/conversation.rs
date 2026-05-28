use protocol::{ConversationId, GroupId, IdentityId, PersonId, ProfileId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "system" => Some(Self::System),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredMessage {
    pub timestamp: i64,
    pub role: MessageRole,
    pub content: String,
    pub identity: Option<IdentityId>,
    pub profile: Option<ProfileId>,
    pub person: Option<PersonId>,
    pub source_gateway_id: Option<String>,
    pub source_message_id: Option<String>,
    pub sender_external_id: Option<String>,
    pub reply_external_id: Option<String>,
    pub metadata: Value,
}

impl StoredMessage {
    pub fn readable_message_id(&self) -> String {
        if let Some(source_message_id) = self
            .source_message_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            return source_message_id.to_string();
        }

        let identity = self.identity.as_ref().map(|id| id.0.as_str()).unwrap_or("");
        let profile = self.profile.as_ref().map(|id| id.0.as_str()).unwrap_or("");
        let person = self.person.as_ref().map(|id| id.0.as_str()).unwrap_or("");
        let source_gateway = self.source_gateway_id.as_deref().unwrap_or("");
        let sender = self.sender_external_id.as_deref().unwrap_or("");
        let reply = self.reply_external_id.as_deref().unwrap_or("");
        let metadata = if self.metadata.is_null() {
            String::new()
        } else {
            self.metadata.to_string()
        };
        let hash_input = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.timestamp,
            self.role.as_str(),
            identity,
            profile,
            person,
            source_gateway,
            sender,
            reply,
            metadata,
            self.content
        );
        format!(
            "local:{}:{}:{}",
            self.role.as_str(),
            self.timestamp,
            stable_local_message_hash(&hash_input)
        )
    }
}

fn stable_local_message_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub gateway_id: Option<String>,
    pub identity: Option<IdentityId>,
    pub profile: Option<ProfileId>,
    pub person: Option<PersonId>,
    pub group: Option<GroupId>,
    pub summary: Option<String>,
    pub summary_covered_message_ids: Vec<String>,
    pub summary_updated_at: Option<i64>,
    pub summary_version: u32,
    pub message_count: u32,
    pub started_at: i64,
    pub last_message_at: i64,
}
