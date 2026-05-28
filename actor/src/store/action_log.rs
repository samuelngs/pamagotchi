use protocol::{ConversationId, MemoryId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionRunRecord {
    pub action_id: String,
    pub kind: String,
    pub task: String,
    pub conversation: Option<ConversationId>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: String,
    pub responded: bool,
    pub attempts: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionTurnRecord {
    pub action_id: String,
    pub turn: u32,
    pub attempt: u32,
    pub prompt_hash: String,
    pub model: Option<String>,
    pub finish: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub text_len: u32,
    pub reasoning_len: u32,
    pub tool_call_count: u32,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionPromptSnapshotRecord {
    pub action_id: String,
    pub turn: u32,
    pub attempt: u32,
    pub prompt_hash: String,
    pub messages: Value,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub action_id: String,
    pub turn: u32,
    pub call_id: String,
    pub name: String,
    pub args: Value,
    pub result: Value,
    pub success: bool,
    pub started_at: i64,
    pub ended_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionMessageRecord {
    pub action_id: String,
    pub role: String,
    pub conversation: Option<ConversationId>,
    pub source_gateway_id: Option<String>,
    pub source_message_id: Option<String>,
    pub sender_external_id: Option<String>,
    pub reply_external_id: Option<String>,
    pub content: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundDeliveryRecord {
    pub action_id: String,
    pub conversation: Option<ConversationId>,
    pub gateway_id: String,
    pub external_id: String,
    pub status: String,
    pub error: Option<String>,
    pub attempted_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionTranscriptRecord {
    pub run: Option<ActionRunRecord>,
    pub turns: Vec<ActionTurnRecord>,
    pub prompt_snapshots: Vec<ActionPromptSnapshotRecord>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub messages: Vec<ActionMessageRecord>,
    pub deliveries: Vec<OutboundDeliveryRecord>,
    pub memories_formed: Vec<MemoryId>,
    pub recalled_memory_ids: Vec<MemoryId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewJobRecord {
    pub source_action_id: String,
    pub review_action_id: String,
    pub scheduled_at: i64,
    pub source_kind: Option<String>,
    pub source_status: Option<String>,
    pub source_started_at: Option<i64>,
    pub source_ended_at: Option<i64>,
    pub review_status: Option<String>,
    pub review_started_at: Option<i64>,
    pub review_ended_at: Option<i64>,
    pub output_count: u32,
    pub last_applied_at: Option<i64>,
}
