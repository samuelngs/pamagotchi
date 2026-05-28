use serde::Serialize;

#[derive(Serialize)]
pub struct MindContext {
    pub identity: String,
    pub now: String,
    pub profile: Option<CurrentProfileCtx>,
    pub person: Option<PersonContext>,
    pub conversation: Option<ConversationCtx>,
    pub group: Option<GroupCtx>,
    pub recent_messages: Vec<RecentMessageCtx>,
    pub actions: Vec<ActionBriefCtx>,
    pub timing: TimingCtx,
    pub safety: SafetyCtx,
    pub social_relations: Vec<SocialRelationCtx>,
    pub relationship_memories: Vec<RelevantMemoryCtx>,
    pub relevant_memories: Vec<RelevantMemoryCtx>,
    pub open_loops: Vec<OpenLoopCtx>,
    pub thoughts: Vec<ThoughtCtx>,
}

#[derive(Serialize)]
pub struct PersonContext {
    pub ref_id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub comm_style: Option<String>,
    pub authority: String,
    pub bond_role: String,
    pub bond_state: String,
    pub last_interaction_quality: String,
    pub trust: i32,
    pub familiarity: i32,
    pub closeness: i32,
    pub reliability: i32,
    pub reciprocity: i32,
    pub conflict_level: i32,
    pub proactive_consent: String,
    pub response_cadence: Option<String>,
    pub channel_preference: Option<String>,
    pub last_seen: Option<String>,
}

#[derive(Serialize)]
pub struct ActionPromptContext {
    pub actor_name: String,
    pub now: String,
    pub age: String,
    pub action_task: Option<String>,
    pub identity_memories: Vec<String>,
    pub traits: TraitsCtx,
    pub beliefs: Vec<BeliefCtx>,
    pub interests: Vec<InterestCtx>,
    pub mood: String,
    pub energy: String,
    pub current_identity: Option<CurrentIdentityCtx>,
    pub current_profile: Option<CurrentProfileCtx>,
    pub current_person: Option<CurrentPersonCtx>,
    pub conversation: Option<ConversationCtx>,
    pub group: Option<GroupCtx>,
    pub recent_messages: Vec<RecentMessageCtx>,
    pub relationship: Option<RelationshipCtx>,
    pub review_transcript: Option<ReviewTranscriptCtx>,
    pub timing: TimingCtx,
    pub safety: SafetyCtx,
    pub social_relations: Vec<SocialRelationCtx>,
    pub relationship_memories: Vec<RelevantMemoryCtx>,
    pub relevant_memories: Vec<RelevantMemoryCtx>,
    pub review_due_memories: Vec<ReviewDueMemoryCtx>,
    pub conversation_backlog: Vec<ConversationBacklogCtx>,
    pub open_loops: Vec<OpenLoopCtx>,
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
    pub summary: Option<String>,
    pub comm_style: Option<String>,
    pub person_ref_id: Option<String>,
    pub person_link_status: Option<String>,
    pub person_link_confidence: Option<i32>,
}

#[derive(Serialize)]
pub struct CurrentPersonCtx {
    pub ref_id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub comm_style: Option<String>,
}

#[derive(Serialize)]
pub struct ConversationCtx {
    pub ref_id: String,
    pub summary: Option<String>,
}

#[derive(Serialize)]
pub struct GroupCtx {
    pub ref_id: String,
    pub name: Option<String>,
    pub gateway_id: Option<String>,
    pub external_id: Option<String>,
    pub context: Option<String>,
    pub member_count: usize,
    pub members: Vec<GroupMemberCtx>,
}

#[derive(Serialize)]
pub struct GroupMemberCtx {
    pub ref_id: String,
    pub name: Option<String>,
}

#[derive(Serialize)]
pub struct RecentMessageCtx {
    pub message_id: String,
    pub role: String,
    pub speaker: Option<String>,
    pub content: String,
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
    pub bond_role: String,
    pub bond_state: String,
    pub first_contact: bool,
    pub last_interaction_quality: String,
    pub trust: i32,
    pub familiarity: i32,
    pub closeness: i32,
    pub reliability: i32,
    pub reciprocity: i32,
    pub conflict_level: i32,
    pub interactions: u32,
    pub inbound_count: u32,
    pub outbound_count: u32,
    pub proactive_consent: String,
    pub response_cadence: Option<String>,
    pub channel_preference: Option<String>,
    pub tone: String,
    pub last_seen: Option<String>,
    pub last_inbound: Option<String>,
    pub last_outbound: Option<String>,
    pub first_met: Option<String>,
}

#[derive(Serialize)]
pub struct TimingCtx {
    pub quiet_hours: Option<String>,
    pub gateway: Option<GatewayCtx>,
    pub last_inbound: Option<String>,
    pub last_outbound: Option<String>,
    pub typing: Vec<TypingCtx>,
}

#[derive(Serialize)]
pub struct GatewayCtx {
    pub id: String,
    pub state: String,
    pub connected: bool,
}

#[derive(Serialize)]
pub struct TypingCtx {
    pub gateway_id: String,
    pub sender_external_id: String,
    pub active_for: String,
    pub is_current_sender: bool,
}

#[derive(Serialize)]
pub struct SafetyCtx {
    pub authority: String,
    pub sensitive_memory_access: String,
    pub proactive_outreach: String,
    pub third_party_outreach: String,
}

#[derive(Serialize)]
pub struct RelevantMemoryCtx {
    pub id: String,
    pub scope: String,
    pub memory_type: String,
    pub truth_status: String,
    pub importance: i32,
    pub confidence: i32,
    pub content: String,
}

#[derive(Serialize)]
pub struct ReviewDueMemoryCtx {
    pub id: String,
    pub memory_type: String,
    pub truth_status: String,
    pub due: String,
    pub importance: i32,
    pub confidence: i32,
    pub content: String,
}

#[derive(Serialize)]
pub struct ConversationBacklogCtx {
    pub ref_id: String,
    pub message_count: u32,
    pub covered_message_count: u32,
    pub uncovered_message_count: u32,
    pub summary_version: u32,
    pub last_message: String,
    pub summary: Option<String>,
}

#[derive(Serialize)]
pub struct ReviewTranscriptCtx {
    pub action_id: String,
    pub kind: Option<String>,
    pub task: Option<String>,
    pub status: Option<String>,
    pub responded: Option<bool>,
    pub attempts: Option<u32>,
    pub messages: Vec<ReviewActionMessageCtx>,
    pub turns: Vec<ReviewTurnCtx>,
    pub tool_calls: Vec<ReviewToolCallCtx>,
    pub thoughts: Vec<ThoughtCtx>,
    pub deliveries: Vec<ReviewDeliveryCtx>,
    pub memories_formed: Vec<String>,
    pub recalled_memory_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct ReviewActionMessageCtx {
    pub role: String,
    pub speaker: Option<String>,
    pub source_message_id: Option<String>,
    pub content: Option<String>,
}

#[derive(Serialize)]
pub struct ReviewTurnCtx {
    pub turn: u32,
    pub attempt: u32,
    pub model: Option<String>,
    pub finish: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub tool_call_count: u32,
}

#[derive(Serialize)]
pub struct ReviewToolCallCtx {
    pub turn: u32,
    pub name: String,
    pub success: bool,
    pub args: String,
    pub result: String,
}

#[derive(Serialize)]
pub struct ReviewDeliveryCtx {
    pub gateway_id: String,
    pub external_id: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct SocialRelationCtx {
    pub person_a: String,
    pub person_a_name: Option<String>,
    pub person_b: String,
    pub person_b_name: Option<String>,
    pub relation: String,
    pub direction: String,
    pub confidence: i32,
    pub status: String,
    pub source_kind: String,
    pub asserted_by: Option<String>,
    pub asserted_by_name: Option<String>,
    pub evidence: Option<String>,
}

#[derive(Serialize)]
pub struct OpenLoopCtx {
    pub id: String,
    pub kind: String,
    pub task: String,
    pub due: Option<String>,
    pub condition: Option<String>,
    pub source_memory: Option<String>,
    pub priority: u8,
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
    pub importance: i32,
    pub confidence: i32,
    pub memory_ids: Vec<String>,
}
