use crate::state::{ActorState, GrowthConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorSnapshot {
    pub state: ActorState,
    pub config: GrowthConfig,
    pub saved_at: i64,
    #[serde(default)]
    pub last_state_journal_id: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateJournalRecord {
    pub id: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: i64,
}
