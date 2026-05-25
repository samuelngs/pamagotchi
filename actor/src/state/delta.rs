use protocol::PersonId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Delta {
    pub trait_nudges: Vec<TraitNudge>,
    pub belief_changes: Vec<BeliefChange>,
    pub relationship_changes: Vec<RelationshipChange>,
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
    pub familiarity_delta: f32,
    pub valence_delta: f32,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AffectShift {
    pub valence: f32,
    pub arousal: f32,
    pub dominance: f32,
}
