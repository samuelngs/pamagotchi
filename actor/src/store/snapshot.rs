use crate::personality::{GrowthConfig, PersonalityState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorSnapshot {
    pub personality: PersonalityState,
    pub config: GrowthConfig,
    pub saved_at: i64,
}
