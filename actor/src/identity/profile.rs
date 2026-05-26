use protocol::{IdentityId, PersonId, ProfileId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub display_name: Option<String>,
    pub summary: Option<String>,
    pub comm_style: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileIdentityLink {
    pub profile_id: ProfileId,
    pub identity_id: IdentityId,
    pub status: ProfileIdentityStatus,
    pub confidence: f32,
    pub evidence: Option<serde_json::Value>,
    pub created_at: i64,
    pub removed_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProfileIdentityStatus {
    Active,
    Removed,
}

impl ProfileIdentityStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Removed => "removed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonProfileLink {
    pub person_id: PersonId,
    pub profile_id: ProfileId,
    pub status: PersonProfileStatus,
    pub confidence: f32,
    pub evidence: Option<serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64,
    pub detached_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PersonProfileStatus {
    Verified,
    Likely,
    Suspected,
    Detached,
    Rejected,
}

impl PersonProfileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Likely => "likely",
            Self::Suspected => "suspected",
            Self::Detached => "detached",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "verified" => Some(Self::Verified),
            "likely" => Some(Self::Likely),
            "suspected" => Some(Self::Suspected),
            "detached" => Some(Self::Detached),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }

    pub fn is_active_person_context(&self) -> bool {
        matches!(self, Self::Verified | Self::Likely)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedActorIdentity {
    pub identity: crate::identity::Identity,
    pub profile: Profile,
    pub person: Option<crate::identity::Person>,
    pub profile_person_link: Option<PersonProfileLink>,
}
