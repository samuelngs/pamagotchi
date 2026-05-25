use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct CoreTraits {
    pub openness: f32,
    pub warmth: f32,
    pub assertiveness: f32,
    pub humor: f32,
    pub curiosity: f32,
    pub patience: f32,
    pub directness: f32,
    pub playfulness: f32,
}

impl CoreTraits {
    pub fn nudge(&mut self, name: &str, amount: f32, floors: &CoreTraits) {
        let (value, floor) = match name {
            "openness" => (&mut self.openness, floors.openness),
            "warmth" => (&mut self.warmth, floors.warmth),
            "assertiveness" => (&mut self.assertiveness, floors.assertiveness),
            "humor" => (&mut self.humor, floors.humor),
            "curiosity" => (&mut self.curiosity, floors.curiosity),
            "patience" => (&mut self.patience, floors.patience),
            "directness" => (&mut self.directness, floors.directness),
            "playfulness" => (&mut self.playfulness, floors.playfulness),
            _ => return,
        };
        *value = (*value + amount).clamp(floor, 1.0);
    }
}

impl Default for CoreTraits {
    fn default() -> Self {
        Self {
            openness: 0.5,
            warmth: 0.5,
            assertiveness: 0.5,
            humor: 0.5,
            curiosity: 0.5,
            patience: 0.5,
            directness: 0.5,
            playfulness: 0.5,
        }
    }
}
