use crate::identity::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Belief {
    pub topic: String,
    pub stance: String,
    pub confidence: f32,
    pub formed_at: i64,
    pub last_challenged: Option<i64>,
    pub about: Option<PersonId>,
}
