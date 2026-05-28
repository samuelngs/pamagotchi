use protocol::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SocialRelation {
    pub person_a: PersonId,
    pub person_b: PersonId,
    pub relation: Relation,
    #[serde(default)]
    pub direction: RelationDirection,
    pub confidence: f32,
    pub status: RelationStatus,
    pub evidence: Option<serde_json::Value>,
    pub source_kind: RelationSource,
    pub asserted_by: Option<PersonId>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SocialRelation {
    pub fn new(person_a: PersonId, person_b: PersonId, relation: Relation) -> Self {
        let direction = relation.default_direction();
        Self {
            person_a,
            person_b,
            relation,
            direction,
            confidence: 0.5,
            status: RelationStatus::Stated,
            evidence: None,
            source_kind: RelationSource::System,
            asserted_by: None,
            created_at: 0,
            updated_at: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationDirection {
    #[default]
    Bidirectional,
    AToB,
    BToA,
}

impl RelationDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bidirectional => "bidirectional",
            Self::AToB => "a_to_b",
            Self::BToA => "b_to_a",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bidirectional" => Some(Self::Bidirectional),
            "a_to_b" => Some(Self::AToB),
            "b_to_a" => Some(Self::BToA),
            _ => None,
        }
    }
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

    pub fn default_direction(&self) -> RelationDirection {
        match self {
            Self::Sibling | Self::Partner | Self::Coworker | Self::Friend => {
                RelationDirection::Bidirectional
            }
            Self::Parent | Self::Child | Self::Custom(_) => RelationDirection::AToB,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelationStatus {
    Hypothesis,
    Stated,
    Confirmed,
    Denied,
    Outdated,
}

impl RelationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hypothesis => "hypothesis",
            Self::Stated => "stated",
            Self::Confirmed => "confirmed",
            Self::Denied => "denied",
            Self::Outdated => "outdated",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "hypothesis" => Self::Hypothesis,
            "confirmed" => Self::Confirmed,
            "denied" => Self::Denied,
            "outdated" => Self::Outdated,
            _ => Self::Stated,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelationSource {
    Inferred,
    Stated,
    OwnerConfirmed,
    Import,
    System,
}

impl RelationSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Stated => "stated",
            Self::OwnerConfirmed => "owner_confirmed",
            Self::Import => "import",
            Self::System => "system",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "inferred" => Self::Inferred,
            "stated" => Self::Stated,
            "owner_confirmed" => Self::OwnerConfirmed,
            "import" => Self::Import,
            _ => Self::System,
        }
    }
}
