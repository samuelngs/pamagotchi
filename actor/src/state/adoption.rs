use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdoptionCandidate {
    pub state: AdoptionRitualState,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionRitualState {
    FirstContactAdoptionClaim,
    AdoptionResisted,
    AdoptionAcceptedIntroPending,
    PreAdoptionRequestRedirect,
    IntroReceivedCertificate,
    AdoptionComplete,
}

impl AdoptionRitualState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FirstContactAdoptionClaim => "first_contact_adoption_claim",
            Self::AdoptionResisted => "adoption_resisted",
            Self::AdoptionAcceptedIntroPending => "adoption_accepted_intro_pending",
            Self::PreAdoptionRequestRedirect => "pre_adoption_request_redirect",
            Self::IntroReceivedCertificate => "intro_received_certificate",
            Self::AdoptionComplete => "adoption_complete",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "first_contact_adoption_claim" => Some(Self::FirstContactAdoptionClaim),
            "adoption_resisted" => Some(Self::AdoptionResisted),
            "adoption_accepted_intro_pending" => Some(Self::AdoptionAcceptedIntroPending),
            "pre_adoption_request_redirect" => Some(Self::PreAdoptionRequestRedirect),
            "intro_received_certificate" => Some(Self::IntroReceivedCertificate),
            "adoption_complete" => Some(Self::AdoptionComplete),
            _ => None,
        }
    }
}
