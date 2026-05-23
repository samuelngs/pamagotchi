use super::delta::AffectShift;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct AffectState {
    pub valence: f32,
    pub arousal: f32,
    pub dominance: f32,
    pub baseline_valence: f32,
    pub baseline_arousal: f32,
    pub baseline_dominance: f32,
}

impl AffectState {
    pub fn shift(&mut self, delta: &AffectShift) {
        self.valence = (self.valence + delta.valence).clamp(-1.0, 1.0);
        self.arousal = (self.arousal + delta.arousal).clamp(0.0, 1.0);
        self.dominance = (self.dominance + delta.dominance).clamp(0.0, 1.0);
    }

    pub fn mean_revert(&mut self, elapsed_secs: f64) {
        let rate = (1.0 - (-0.001 * elapsed_secs).exp()) as f32;
        self.valence += (self.baseline_valence - self.valence) * rate;
        self.arousal += (self.baseline_arousal - self.arousal) * rate;
        self.dominance += (self.baseline_dominance - self.dominance) * rate;
    }
}

impl Default for AffectState {
    fn default() -> Self {
        Self {
            valence: 0.3,
            arousal: 0.3,
            dominance: 0.4,
            baseline_valence: 0.3,
            baseline_arousal: 0.3,
            baseline_dominance: 0.4,
        }
    }
}
