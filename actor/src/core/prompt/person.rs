use super::format::{pct, relative_duration};
use super::*;

pub(super) async fn resolve_person_for_mind(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
) -> Option<PersonContext> {
    let msg = messages.first()?;
    let person_id = msg.person.as_ref()?;
    let info = resolve_person_info(store, person_id).await;
    let actor = state.read_state();
    let now_unix = chrono::Utc::now().timestamp();
    let last_seen = info.last_seen.map(|ts| relative_duration(ts, now_unix));
    if let Some(rel) = actor.bonds.get(person_id) {
        Some(PersonContext {
            ref_id: person_id.0.clone(),
            name: info.name,
            summary: info.summary,
            comm_style: info.comm_style,
            relationship_standing: rel.relationship_standing.as_str().to_string(),
            bond_role: bond_role(&rel.relationship_standing).into(),
            bond_state: bond_state(rel).into(),
            last_interaction_quality: interaction_quality(rel).into(),
            trust: pct(rel.trust),
            familiarity: pct(rel.familiarity),
            closeness: pct(rel.closeness),
            reliability: pct(rel.reliability),
            reciprocity: pct(rel.reciprocity),
            conflict_level: pct(rel.conflict_level),
            proactive_consent: rel.proactive_consent.as_str().into(),
            response_cadence: rel.response_cadence.clone(),
            channel_preference: rel.channel_preference.clone(),
            last_seen,
        })
    } else {
        Some(PersonContext {
            ref_id: person_id.0.clone(),
            name: info.name,
            summary: info.summary,
            comm_style: info.comm_style,
            relationship_standing: "default".into(),
            bond_role: "new_person".into(),
            bond_state: "unfamiliar".into(),
            last_interaction_quality: "unknown".into(),
            trust: 0,
            familiarity: 0,
            closeness: 0,
            reliability: 0,
            reciprocity: 0,
            conflict_level: 0,
            proactive_consent: "unknown".into(),
            response_cadence: None,
            channel_preference: None,
            last_seen,
        })
    }
}

pub(super) struct PersonInfo {
    pub(super) name: Option<String>,
    pub(super) summary: Option<String>,
    pub(super) comm_style: Option<String>,
    pub(super) first_seen: Option<i64>,
    pub(super) last_seen: Option<i64>,
}

pub(super) async fn resolve_person_info(
    store: &Arc<dyn Store>,
    person_id: &protocol::PersonId,
) -> PersonInfo {
    match store.get_person(person_id).await {
        Ok(Some(p)) => PersonInfo {
            name: p.name,
            summary: p.summary,
            comm_style: p.comm_style,
            first_seen: Some(p.first_seen),
            last_seen: Some(p.last_seen),
        },
        _ => PersonInfo {
            name: None,
            summary: None,
            comm_style: None,
            first_seen: None,
            last_seen: None,
        },
    }
}

pub(super) fn bond_role(relationship_standing: &RelationshipStanding) -> &'static str {
    match relationship_standing {
        RelationshipStanding::ChosenHuman => "chosen_human",
        RelationshipStanding::Trusted => "trusted_person",
        RelationshipStanding::Default => "current_person",
        RelationshipStanding::Restricted => "guarded_person",
        RelationshipStanding::Blocked => "blocked_person",
    }
}

pub(super) fn bond_state(rel: &crate::state::Relationship) -> &'static str {
    if matches!(rel.relationship_standing, RelationshipStanding::Blocked) {
        "blocked"
    } else if rel.conflict_level > 0.45 || rel.emotional_valence < -0.45 {
        "strained"
    } else if rel.inbound_count <= 1 && rel.familiarity < 0.05 {
        "first_contact"
    } else if rel.closeness >= 0.65 || rel.familiarity >= 0.65 {
        "bonded"
    } else if rel.familiarity >= 0.25 || rel.closeness >= 0.25 {
        "warming"
    } else {
        "acquaintance"
    }
}

pub(super) fn interaction_quality(rel: &crate::state::Relationship) -> &'static str {
    if rel.emotional_valence > 0.3 {
        "warm"
    } else if rel.emotional_valence < -0.3 || rel.conflict_level > 0.3 {
        "strained"
    } else if rel.interaction_count == 0 {
        "unknown"
    } else {
        "neutral"
    }
}
