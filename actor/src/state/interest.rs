use protocol::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Interest {
    pub topic: String,
    pub intensity: f32,
    pub origin: String,
    pub origin_person: Option<PersonId>,
    pub last_engaged: i64,
}

impl Interest {
    pub fn decay(&mut self, elapsed_secs: f64) {
        self.intensity *= (-0.0001 * elapsed_secs).exp() as f32;
    }
}
