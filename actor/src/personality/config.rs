use super::CoreTraits;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct GrowthConfig {
    pub rate: GrowthRate,
    pub core_trait_inertia: f32,
    pub trait_floors: CoreTraits,
}

impl GrowthConfig {
    pub fn rate_multiplier(&self) -> f32 {
        self.rate.multiplier()
    }
}

impl Default for GrowthConfig {
    fn default() -> Self {
        Self {
            rate: GrowthRate::Normal,
            core_trait_inertia: 0.95,
            trait_floors: CoreTraits {
                openness: 0.0,
                warmth: 0.0,
                assertiveness: 0.2,
                humor: 0.0,
                curiosity: 0.0,
                patience: 0.0,
                directness: 0.15,
                playfulness: 0.0,
            },
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum GrowthRate {
    Slow,
    Normal,
    Fast,
}

impl GrowthRate {
    pub fn multiplier(&self) -> f32 {
        match self {
            GrowthRate::Slow => 0.5,
            GrowthRate::Normal => 1.0,
            GrowthRate::Fast => 2.0,
        }
    }
}
