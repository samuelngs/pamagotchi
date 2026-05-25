use protocol::PersonId;
use crate::state::{AffectShift, Delta};

pub fn empty_delta(triggered_by: Option<PersonId>) -> Delta {
    Delta {
        trait_nudges: vec![],
        belief_changes: vec![],
        relationship_changes: vec![],
        new_interests: vec![],
        affect_shift: AffectShift::default(),
        growth_note: None,
        triggered_by,
    }
}

pub fn has_changes(delta: &Delta) -> bool {
    !delta.trait_nudges.is_empty()
        || !delta.belief_changes.is_empty()
        || !delta.relationship_changes.is_empty()
        || !delta.new_interests.is_empty()
        || delta.growth_note.is_some()
        || delta.affect_shift.valence != 0.0
        || delta.affect_shift.arousal != 0.0
        || delta.affect_shift.dominance != 0.0
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", t)
}
