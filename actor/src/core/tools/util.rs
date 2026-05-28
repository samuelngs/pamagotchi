use crate::state::{AffectShift, Delta};
use protocol::PersonId;
use serde_json::Value;

pub fn empty_delta(triggered_by: Option<PersonId>) -> Delta {
    Delta {
        trait_nudges: vec![],
        belief_changes: vec![],
        relationship_changes: vec![],
        relationship_signal_updates: vec![],
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
        || !delta.relationship_signal_updates.is_empty()
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

pub fn evidence_with_source_spans(item: &Value, default: Value) -> Value {
    let mut evidence = item
        .get("evidence")
        .or_else(|| item.get("evidence_json"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(default);
    let source_spans = source_spans(item);
    if source_spans.is_empty() {
        return evidence;
    }

    if let Value::Object(object) = &mut evidence {
        object.insert("source_spans".into(), Value::Array(source_spans));
        evidence
    } else {
        serde_json::json!({
            "evidence": evidence,
            "source_spans": source_spans,
        })
    }
}

fn source_spans(item: &Value) -> Vec<Value> {
    let mut spans = item["source_spans"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|span| span.is_object())
        .cloned()
        .collect::<Vec<_>>();
    if let Some(span) = item.get("source_span").filter(|span| span.is_object()) {
        spans.push(span.clone());
    }
    spans
}
