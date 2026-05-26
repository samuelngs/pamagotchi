use protocol::IdentityId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub id: IdentityId,
    pub gateway_id: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: i64,
    pub last_seen_at: i64,
}
