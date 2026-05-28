use super::ProactiveConsent;
use protocol::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Delta {
    pub trait_nudges: Vec<TraitNudge>,
    pub belief_changes: Vec<BeliefChange>,
    pub relationship_changes: Vec<RelationshipChange>,
    #[serde(default)]
    pub relationship_signal_updates: Vec<RelationshipSignalUpdate>,
    pub new_interests: Vec<String>,
    pub affect_shift: AffectShift,
    pub growth_note: Option<String>,
    pub triggered_by: Option<PersonId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TraitNudge {
    pub trait_name: String,
    pub direction: f32,
    pub reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BeliefChange {
    pub topic: String,
    pub new_stance: Option<String>,
    pub confidence_delta: f32,
    pub reason: String,
    pub about: Option<PersonId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RelationshipChange {
    pub person: PersonId,
    pub trust_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_ceiling: Option<f32>,
    pub familiarity_delta: f32,
    pub valence_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proactive_consent: Option<ProactiveConsent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_cadence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_preference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<RelationshipInteraction>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RelationshipSignalUpdate {
    pub person: PersonId,
    pub closeness_delta: f32,
    pub reliability_delta: f32,
    pub reciprocity_delta: f32,
    pub conflict_delta: f32,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipInteraction {
    Inbound,
    Outbound,
    ProactiveOutbound,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AffectShift {
    pub valence: f32,
    pub arousal: f32,
    pub dominance: f32,
}
