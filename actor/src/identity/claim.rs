use super::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityClaim {
    pub id: String,
    pub claimant: PersonId,
    pub claimed_person: PersonId,
    pub evidence: ClaimEvidence,
    pub confidence: f32,
    pub status: ClaimStatus,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClaimEvidence {
    SelfDeclaration,
    OwnerVouched,
    MutualClaim,
    SharedKnowledge,
    ConfiguredAlias,
}

impl ClaimEvidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SelfDeclaration => "self_declaration",
            Self::OwnerVouched => "owner_vouched",
            Self::MutualClaim => "mutual_claim",
            Self::SharedKnowledge => "shared_knowledge",
            Self::ConfiguredAlias => "configured_alias",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "self_declaration" => Some(Self::SelfDeclaration),
            "owner_vouched" => Some(Self::OwnerVouched),
            "mutual_claim" => Some(Self::MutualClaim),
            "shared_knowledge" => Some(Self::SharedKnowledge),
            "configured_alias" => Some(Self::ConfiguredAlias),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    Pending,
    Confirmed,
    Denied,
    Merged,
}

impl ClaimStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Denied => "denied",
            Self::Merged => "merged",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "confirmed" => Some(Self::Confirmed),
            "denied" => Some(Self::Denied),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }
}
