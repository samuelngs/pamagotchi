use crate::state::{ActorState, GrowthConfig};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorSnapshot {
    pub state: ActorState,
    pub config: GrowthConfig,
    pub saved_at: i64,
}
