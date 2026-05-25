use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Relationship {
    pub authority: Authority,
    pub trust: f32,
    pub familiarity: f32,
    pub emotional_valence: f32,
    pub last_interaction: i64,
    pub interaction_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authority {
    Owner,
    Trusted,
    Default,
    Restricted,
    Blocked,
}

impl Authority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Trusted => "trusted",
            Self::Default => "default",
            Self::Restricted => "restricted",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "trusted" => Some(Self::Trusted),
            "default" => Some(Self::Default),
            "restricted" => Some(Self::Restricted),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

impl Default for Relationship {
    fn default() -> Self {
        Self {
            authority: Authority::Default,
            trust: 0.3,
            familiarity: 0.0,
            emotional_valence: 0.0,
            last_interaction: 0,
            interaction_count: 0,
        }
    }
}
