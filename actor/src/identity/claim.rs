use protocol::PersonId;
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
    ConfiguredIdentity,
}

impl ClaimEvidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SelfDeclaration => "self_declaration",
            Self::OwnerVouched => "owner_vouched",
            Self::MutualClaim => "mutual_claim",
            Self::SharedKnowledge => "shared_knowledge",
            Self::ConfiguredIdentity => "configured_identity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "self_declaration" => Some(Self::SelfDeclaration),
            "owner_vouched" => Some(Self::OwnerVouched),
            "mutual_claim" => Some(Self::MutualClaim),
            "shared_knowledge" => Some(Self::SharedKnowledge),
            "configured_identity" => Some(Self::ConfiguredIdentity),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    Pending,
    Confirmed,
    Denied,
    Linked,
}

impl ClaimStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Denied => "denied",
            Self::Linked => "linked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "confirmed" => Some(Self::Confirmed),
            "denied" => Some(Self::Denied),
            "linked" => Some(Self::Linked),
            _ => None,
        }
    }
}
