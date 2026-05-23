use super::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SocialRelation {
    pub person_a: PersonId,
    pub person_b: PersonId,
    pub relation: Relation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Relation {
    Parent,
    Child,
    Sibling,
    Partner,
    Coworker,
    Friend,
    Custom(String),
}

impl Relation {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Parent => "parent",
            Self::Child => "child",
            Self::Sibling => "sibling",
            Self::Partner => "partner",
            Self::Coworker => "coworker",
            Self::Friend => "friend",
            Self::Custom(s) => s,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "parent" => Self::Parent,
            "child" => Self::Child,
            "sibling" => Self::Sibling,
            "partner" => Self::Partner,
            "coworker" => Self::Coworker,
            "friend" => Self::Friend,
            other => Self::Custom(other.to_string()),
        }
    }
}
