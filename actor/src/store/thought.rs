use crate::identity::PersonId;
use super::MemoryId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ThoughtKind {
    Reflection,
    Rumination,
    Consolidation,
    Planning,
    Observation,
}

impl ThoughtKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reflection => "reflection",
            Self::Rumination => "rumination",
            Self::Consolidation => "consolidation",
            Self::Planning => "planning",
            Self::Observation => "observation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reflection" => Some(Self::Reflection),
            "rumination" => Some(Self::Rumination),
            "consolidation" => Some(Self::Consolidation),
            "planning" => Some(Self::Planning),
            "observation" => Some(Self::Observation),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thought {
    pub timestamp: i64,
    pub kind: ThoughtKind,
    pub content: String,
    pub memories_accessed: Vec<MemoryId>,
    pub people: Vec<PersonId>,
}
