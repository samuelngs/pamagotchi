use super::super::context::{SessionContext, SessionState};
use crate::identity::{
    Relation, RelationDirection, RelationSource, RelationStatus, SocialRelation,
};
use protocol::{InboundMessage, PersonId};
use serde_json::{Value, json};

pub async fn upsert_social_relation(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> String {
    let Some(person_a) = args["person_a"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return json!({
            "status": "error",
            "message": "person_a is required.",
        })
        .to_string();
    };
    let Some(person_b) = args["person_b"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return json!({
            "status": "error",
            "message": "person_b is required.",
        })
        .to_string();
    };
    if person_a == person_b {
        return json!({
            "status": "error",
            "message": "Social relations require two distinct people.",
        })
        .to_string();
    }

    let relation = args["relation"]
        .as_str()
        .filter(|relation| !relation.trim().is_empty())
        .map(Relation::parse)
        .unwrap_or_else(|| Relation::Custom("related".into()));
    let direction = args["direction"]
        .as_str()
        .and_then(RelationDirection::parse)
        .unwrap_or_else(|| relation.default_direction());
    let confidence = args["confidence"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) as f32;
    let status = args["status"]
        .as_str()
        .map(RelationStatus::parse)
        .unwrap_or(RelationStatus::Stated);
    let source_kind = args["source_kind"]
        .as_str()
        .map(RelationSource::parse)
        .unwrap_or(RelationSource::Stated);
    if let Some(missing) = missing_relation_evidence_message_ids(args, ctx, state) {
        return json!({
            "status": "error",
            "message": "Explicit social relation evidence_message_ids must reference messages available to the current action.",
            "missing_evidence_message_ids": missing,
        })
        .to_string();
    }
    let asserted_by = relation_asserted_by_person(args, ctx, state, &source_kind);
    let now = super::super::util::now();

    let relation = SocialRelation {
        person_a: person_a.clone(),
        person_b: person_b.clone(),
        relation,
        direction,
        confidence,
        status,
        evidence: Some(relation_evidence(args, ctx, state)),
        source_kind,
        asserted_by,
        created_at: now,
        updated_at: now,
    };

    match ctx.store.upsert_relation(&relation).await {
        Ok(()) => json!({
            "status": "updated",
            "person_a": person_a.0,
            "person_b": person_b.0,
            "relation": relation.relation.as_str(),
            "confidence": relation.confidence,
            "relation_status": relation.status.as_str(),
            "source_kind": relation.source_kind.as_str(),
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

fn relation_evidence(args: &Value, ctx: &SessionContext, state: &SessionState) -> Value {
    let supplied = args
        .get("evidence")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let mut evidence = json!({
        "action_id": ctx.action_id.0,
        "message_ids": relation_evidence_message_ids(args, ctx, state),
        "evidence": supplied,
    });
    if let Some(quote) = args["evidence_quote"]
        .as_str()
        .map(str::trim)
        .filter(|quote| !quote.is_empty())
    {
        evidence["quote"] = json!(quote);
    }
    evidence
}

fn relation_evidence_message_ids(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> Vec<String> {
    let supplied = explicit_relation_evidence_message_ids(args);
    if !supplied.is_empty() {
        return supplied;
    }
    evidence_source_messages(ctx, state)
        .iter()
        .map(|message| message.message_id.clone())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>()
}

fn explicit_relation_evidence_message_ids(args: &Value) -> Vec<String> {
    let supplied = string_array(&args["evidence_message_ids"]).collect::<Vec<_>>();
    if !supplied.is_empty() {
        return supplied;
    }
    if let Some(ids) = args
        .get("evidence")
        .and_then(|evidence| evidence.get("message_ids"))
        .map(|value| string_array(value).collect::<Vec<_>>())
        .filter(|ids| !ids.is_empty())
    {
        return ids;
    }
    vec![]
}

fn missing_relation_evidence_message_ids(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> Option<Vec<String>> {
    let supplied = explicit_relation_evidence_message_ids(args);
    if supplied.is_empty() {
        return None;
    }
    let messages = evidence_source_messages(ctx, state);
    let missing = supplied
        .into_iter()
        .filter(|id| {
            !messages
                .iter()
                .any(|message| message.message_id.as_str() == id.as_str())
        })
        .collect::<Vec<_>>();
    (!missing.is_empty()).then_some(missing)
}

fn relation_asserted_by_person(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    source_kind: &RelationSource,
) -> Option<PersonId> {
    if let Some(person) = args["asserted_by_person_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| PersonId(id.to_string()))
    {
        return Some(person);
    }
    if !matches!(
        source_kind,
        RelationSource::Stated | RelationSource::ChosenPersonConfirmed
    ) {
        return None;
    }
    let evidence_ids = relation_evidence_message_ids(args, ctx, state);
    let messages = evidence_source_messages(ctx, state);
    evidence_ids
        .iter()
        .find_map(|id| messages.iter().find(|message| message.message_id == *id))
        .or_else(|| messages.first())
        .and_then(|message| message.person.clone())
}

fn evidence_source_messages(ctx: &SessionContext, state: &SessionState) -> Vec<InboundMessage> {
    ctx.messages
        .iter()
        .chain(state.presented_injected_messages.iter())
        .chain(state.presented_read_messages.iter())
        .cloned()
        .collect()
}

fn string_array(value: &Value) -> impl Iterator<Item = String> + '_ {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests;
