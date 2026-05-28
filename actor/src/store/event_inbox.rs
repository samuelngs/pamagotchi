use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventInboxRecord {
    pub id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub due_at: i64,
    pub attempts: u32,
    pub dedupe_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub fired_at: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventInboxDebugRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub due_at: i64,
    pub attempts: u32,
    pub dedupe_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub fired_at: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}
