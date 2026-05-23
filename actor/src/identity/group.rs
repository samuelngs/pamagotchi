use super::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub platform_id: String,
    pub external_id: String,
    pub context: GroupContext,
    pub members: Vec<PersonId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GroupContext {
    Family,
    Work,
    Social,
    Custom(String),
}

impl GroupContext {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Family => "family",
            Self::Work => "work",
            Self::Social => "social",
            Self::Custom(s) => s,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "family" => Self::Family,
            "work" => Self::Work,
            "social" => Self::Social,
            other => Self::Custom(other.to_string()),
        }
    }
}
