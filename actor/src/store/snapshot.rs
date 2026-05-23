use crate::identity::PersonId;
use crate::personality::{GrowthConfig, PersonalityState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorConfig {
    pub name: String,
    pub description: String,
    pub owner: PersonId,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ActorSnapshot {
    pub actor: ActorConfig,
    pub personality: PersonalityState,
    pub config: GrowthConfig,
    pub saved_at: i64,
}
