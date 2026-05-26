use serde::Serialize;

#[derive(Serialize)]
pub struct MindContext {
    pub identity: String,
    pub now: String,
    pub person: Option<PersonContext>,
    pub actions: Vec<ActionBriefCtx>,
    pub thoughts: Vec<ThoughtCtx>,
}

#[derive(Serialize)]
pub struct PersonContext {
    pub ref_id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub authority: String,
    pub trust: i32,
    pub familiarity: i32,
    pub last_seen: Option<String>,
}

#[derive(Serialize)]
pub struct ActionPromptContext {
    pub now: String,
    pub age: String,
    pub identity_memories: Vec<String>,
    pub traits: TraitsCtx,
    pub beliefs: Vec<BeliefCtx>,
    pub interests: Vec<InterestCtx>,
    pub mood: String,
    pub energy: String,
    pub current_identity: Option<CurrentIdentityCtx>,
    pub current_profile: Option<CurrentProfileCtx>,
    pub current_person: Option<CurrentPersonCtx>,
    pub relationship: Option<RelationshipCtx>,
    pub directives: Vec<String>,
    pub thoughts: Vec<ThoughtCtx>,
    pub cancelled_note: Option<String>,
    pub concurrent_actions: Vec<ActionBriefCtx>,
    pub style: Option<String>,
    pub authority: String,
    pub kind: String,
}

#[derive(Serialize)]
pub struct CurrentIdentityCtx {
    pub ref_id: String,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct CurrentProfileCtx {
    pub ref_id: String,
    pub display_name: Option<String>,
    pub person_ref_id: Option<String>,
    pub person_link_status: Option<String>,
    pub person_link_confidence: Option<i32>,
}

#[derive(Serialize)]
pub struct CurrentPersonCtx {
    pub ref_id: String,
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct TraitsCtx {
    pub openness: i32,
    pub warmth: i32,
    pub assertiveness: i32,
    pub humor: i32,
    pub curiosity: i32,
    pub patience: i32,
    pub directness: i32,
    pub playfulness: i32,
}

#[derive(Serialize)]
pub struct BeliefCtx {
    pub topic: String,
    pub about: Option<String>,
    pub stance: String,
    pub confidence: i32,
}

#[derive(Serialize)]
pub struct InterestCtx {
    pub topic: String,
    pub intensity: i32,
}

#[derive(Serialize)]
pub struct RelationshipCtx {
    pub ref_id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub trust: i32,
    pub familiarity: i32,
    pub interactions: u32,
    pub tone: String,
    pub last_seen: Option<String>,
    pub first_met: Option<String>,
}

#[derive(Serialize)]
pub struct ActionBriefCtx {
    pub id: String,
    pub kind: String,
    pub task: String,
}

#[derive(Serialize)]
pub struct ThoughtCtx {
    pub kind: String,
    pub content: String,
}
