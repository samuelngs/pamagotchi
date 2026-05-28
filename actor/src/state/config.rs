use super::CoreTraits;
use chrono::Timelike;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct GrowthConfig {
    pub rate: GrowthRate,
    pub core_trait_inertia: f32,
    pub trait_floors: CoreTraits,
    #[serde(default)]
    pub proactivity: ProactivityConfig,
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
            proactivity: ProactivityConfig::default(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProactivityConfig {
    pub quiet_hours_utc: Option<QuietHoursUtc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct QuietHoursUtc {
    pub start_hour: u8,
    pub end_hour: u8,
}

impl QuietHoursUtc {
    pub fn delay_until_end(&self, now: chrono::DateTime<chrono::Utc>) -> Option<u64> {
        let start = hour_to_secs(self.start_hour)?;
        let end = hour_to_secs(self.end_hour)?;
        if start == end {
            return None;
        }

        let current = now.hour() * 3600 + now.minute() * 60 + now.second();
        let delay = if start < end {
            (current >= start && current < end).then_some(end - current)
        } else if current >= start {
            Some(24 * 3600 - current + end)
        } else if current < end {
            Some(end - current)
        } else {
            None
        }?;

        Some(delay.max(60) as u64)
    }
}

fn hour_to_secs(hour: u8) -> Option<u32> {
    (hour < 24).then_some(hour as u32 * 3600)
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn quiet_hours_delay_handles_overnight_window() {
        let quiet = QuietHoursUtc {
            start_hour: 22,
            end_hour: 7,
        };
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 27, 23, 30, 0)
            .unwrap();

        assert_eq!(quiet.delay_until_end(now), Some(7 * 3600 + 30 * 60));
    }

    #[test]
    fn quiet_hours_delay_is_none_outside_window() {
        let quiet = QuietHoursUtc {
            start_hour: 22,
            end_hour: 7,
        };
        let now = chrono::Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();

        assert_eq!(quiet.delay_until_end(now), None);
    }
}
