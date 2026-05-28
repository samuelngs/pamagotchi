use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Relationship {
    pub authority: Authority,
    pub trust: f32,
    pub familiarity: f32,
    pub emotional_valence: f32,
    #[serde(default)]
    pub proactive_consent: ProactiveConsent,
    #[serde(default)]
    pub response_cadence: Option<String>,
    #[serde(default)]
    pub channel_preference: Option<String>,
    pub last_interaction: i64,
    pub interaction_count: u32,
    #[serde(default)]
    pub last_inbound: i64,
    #[serde(default)]
    pub last_outbound: i64,
    #[serde(default)]
    pub inbound_count: u32,
    #[serde(default)]
    pub outbound_count: u32,
    #[serde(default)]
    pub last_proactive_outbound: i64,
    #[serde(default)]
    pub proactive_outbound_count: u32,
    #[serde(default)]
    pub closeness: f32,
    #[serde(default)]
    pub reliability: f32,
    #[serde(default)]
    pub reciprocity: f32,
    #[serde(default)]
    pub conflict_level: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProactiveConsent {
    #[default]
    Unknown,
    Allowed,
    Denied,
}

impl ProactiveConsent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unknown" => Some(Self::Unknown),
            "allowed" => Some(Self::Allowed),
            "denied" => Some(Self::Denied),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authority {
    ChosenPerson,
    Trusted,
    Default,
    Restricted,
    Blocked,
}

impl Authority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChosenPerson => "chosen_person",
            Self::Trusted => "trusted",
            Self::Default => "default",
            Self::Restricted => "restricted",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chosen_person" => Some(Self::ChosenPerson),
            "trusted" => Some(Self::Trusted),
            "default" => Some(Self::Default),
            "restricted" => Some(Self::Restricted),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    pub fn trust_ceiling(&self) -> f32 {
        match self {
            Self::ChosenPerson => 1.0,
            Self::Trusted => 0.9,
            Self::Default => 0.6,
            Self::Restricted => 0.2,
            Self::Blocked => 0.0,
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
            proactive_consent: ProactiveConsent::Unknown,
            response_cadence: None,
            channel_preference: None,
            last_interaction: 0,
            interaction_count: 0,
            last_inbound: 0,
            last_outbound: 0,
            inbound_count: 0,
            outbound_count: 0,
            last_proactive_outbound: 0,
            proactive_outbound_count: 0,
            closeness: 0.0,
            reliability: 0.0,
            reciprocity: 0.0,
            conflict_level: 0.0,
        }
    }
}
