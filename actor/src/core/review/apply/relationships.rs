use super::*;

pub(super) async fn apply_relationship_deltas(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    for (idx, item) in array_items(&args["relationship_delta"]).enumerate() {
        let Some(person_id) = item["person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
        else {
            counts
                .skipped
                .push("relationship_delta missing person_id".into());
            continue;
        };
        let trust_delta = clamp_delta(item["trust_delta"].as_f64().unwrap_or(0.0) as f32, 0.05);
        let familiarity_delta = clamp_delta(
            item["familiarity_delta"].as_f64().unwrap_or(0.0) as f32,
            0.1,
        );
        let valence_delta = clamp_delta(item["valence_delta"].as_f64().unwrap_or(0.0) as f32, 0.1);
        let closeness_delta =
            clamp_delta(item["closeness_delta"].as_f64().unwrap_or(0.0) as f32, 0.05);
        let reliability_delta = clamp_delta(
            item["reliability_delta"].as_f64().unwrap_or(0.0) as f32,
            0.05,
        );
        let reciprocity_delta = clamp_delta(
            item["reciprocity_delta"].as_f64().unwrap_or(0.0) as f32,
            0.05,
        );
        let conflict_delta =
            clamp_delta(item["conflict_delta"].as_f64().unwrap_or(0.0) as f32, 0.05);
        let proactive_consent = item["proactive_consent"]
            .as_str()
            .and_then(ProactiveConsent::parse);
        let response_cadence = trimmed_text(item["response_cadence"].as_str(), 240);
        let channel_preference = trimmed_text(item["channel_preference"].as_str(), 240);
        let has_preference_update = response_cadence.is_some() || channel_preference.is_some();
        if !relationship_delta_target_allowed(
            ctx,
            state,
            &person_id,
            trust_delta,
            familiarity_delta,
            valence_delta,
            closeness_delta,
            reliability_delta,
            reciprocity_delta,
            conflict_delta,
            proactive_consent.as_ref(),
            has_preference_update,
        )
        .await
        {
            counts.skipped.push(format!(
                "relationship_delta {idx} targets a person without current verified/strong context"
            ));
            continue;
        }
        let dedupe_key = item["dedupe_key"]
            .as_str()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                let consent = proactive_consent
                    .as_ref()
                    .map(ProactiveConsent::as_str)
                    .unwrap_or("none");
                let response_cadence_key = response_cadence
                    .as_deref()
                    .map(stable_hash)
                    .unwrap_or_else(|| "none".into());
                let channel_preference_key = channel_preference
                    .as_deref()
                    .map(stable_hash)
                    .unwrap_or_else(|| "none".into());
                format!(
                    "review:{}:relationship:{}:{}:{trust_delta:.4}:{familiarity_delta:.4}:{valence_delta:.4}:{closeness_delta:.4}:{reliability_delta:.4}:{reciprocity_delta:.4}:{conflict_delta:.4}:{consent}:{response_cadence_key}:{channel_preference_key}:{}",
                    ctx.action_id.0,
                    idx,
                    person_id.0,
                    stable_hash(item["reason"].as_str().unwrap_or(""))
                )
            });
        if !state
            .applied_review_keys
            .insert(format!("relationship:{dedupe_key}"))
        {
            counts
                .skipped
                .push(format!("relationship_delta {idx} duplicate"));
            continue;
        }
        let trust_ceiling = permission::relationship_trust_ceiling(ctx, &person_id).await;
        state.delta.relationship_changes.push(RelationshipChange {
            person: person_id.clone(),
            trust_delta,
            trust_ceiling: Some(trust_ceiling),
            familiarity_delta,
            valence_delta,
            proactive_consent,
            response_cadence,
            channel_preference,
            interaction: None,
        });
        if closeness_delta != 0.0
            || reliability_delta != 0.0
            || reciprocity_delta != 0.0
            || conflict_delta != 0.0
        {
            state
                .delta
                .relationship_signal_updates
                .push(RelationshipSignalUpdate {
                    person: person_id,
                    closeness_delta,
                    reliability_delta,
                    reciprocity_delta,
                    conflict_delta,
                    reason: item["reason"].as_str().unwrap_or("").to_string(),
                });
        }
        counts.relationship_deltas += 1;
    }
}

pub(super) async fn apply_social_relations(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    for (idx, item) in array_items(&args["social_relations"]).enumerate() {
        let Some(person_a) = item["person_a"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
        else {
            counts
                .skipped
                .push(format!("social_relation {idx} missing person_a"));
            continue;
        };
        let Some(person_b) = item["person_b"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
        else {
            counts
                .skipped
                .push(format!("social_relation {idx} missing person_b"));
            continue;
        };
        if person_a == person_b {
            counts
                .skipped
                .push(format!("social_relation {idx} uses one person"));
            continue;
        }

        let relation = item["relation"]
            .as_str()
            .filter(|relation| !relation.trim().is_empty())
            .map(Relation::parse)
            .unwrap_or_else(|| Relation::Custom("related".into()));
        let direction = item["direction"]
            .as_str()
            .and_then(RelationDirection::parse)
            .unwrap_or_else(|| relation.default_direction());
        let relation_key = relation.as_str().to_string();
        let confidence = clamp_unit(item["confidence"].as_f64().unwrap_or(0.5) as f32);
        let status = item["status"]
            .as_str()
            .map(RelationStatus::parse)
            .unwrap_or(RelationStatus::Stated);
        let source_kind = item["source_kind"]
            .as_str()
            .map(RelationSource::parse)
            .unwrap_or(RelationSource::Stated);
        if matches!(source_kind, RelationSource::ChosenHumanConfirmed)
            && !matches!(ctx.authority, crate::state::Authority::ChosenHuman)
        {
            counts.skipped.push(format!(
                "social_relation {idx} chosen-human confirmation denied"
            ));
            continue;
        }
        match permission::social_relation_targets_current_or_verified(item, ctx).await {
            Ok(true) => {}
            Ok(false) => {
                counts.skipped.push(format!(
                    "social_relation {idx} lacks a current verified/strong person anchor"
                ));
                continue;
            }
            Err(e) => {
                counts.skipped.push(format!(
                    "social_relation {idx} target verification failed: {e}"
                ));
                continue;
            }
        }
        let explicit_evidence_ids = explicit_relation_evidence_message_ids(item);
        if let Some(missing) = missing_evidence_message_ids(ctx, state, &explicit_evidence_ids) {
            counts.skipped.push(format!(
                "social_relation {idx} unavailable evidence message ids: {}",
                missing.join(",")
            ));
            continue;
        }

        let evidence = review_relation_evidence(item, ctx, state);
        let asserted_by = relation_asserted_by_person(item, ctx, state, &source_kind);
        let dedupe_key = item["dedupe_key"]
            .as_str()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "review:{}:social:{}:{}:{}:{}:{}:{}:{}",
                    ctx.action_id.0,
                    idx,
                    person_a.0.as_str(),
                    person_b.0.as_str(),
                    relation_key,
                    status.as_str(),
                    source_kind.as_str(),
                    stable_hash(&evidence.to_string())
                )
            });
        if !state
            .applied_review_keys
            .insert(format!("social_relation:{dedupe_key}"))
        {
            counts
                .skipped
                .push(format!("social_relation {idx} duplicate"));
            continue;
        }

        let now = util::now();
        let relation = SocialRelation {
            person_a: person_a.clone(),
            person_b: person_b.clone(),
            relation,
            direction,
            confidence,
            status,
            evidence: Some(evidence),
            source_kind,
            asserted_by,
            created_at: now,
            updated_at: now,
        };
        match ctx.store.upsert_relation(&relation).await {
            Ok(()) => counts.social_relations += 1,
            Err(e) => counts
                .skipped
                .push(format!("social_relation {idx} failed: {e}")),
        }
    }
}

async fn relationship_delta_target_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    person: &PersonId,
    trust_delta: f32,
    familiarity_delta: f32,
    valence_delta: f32,
    closeness_delta: f32,
    reliability_delta: f32,
    reciprocity_delta: f32,
    conflict_delta: f32,
    proactive_consent: Option<&ProactiveConsent>,
    has_preference_update: bool,
) -> bool {
    if matches!(ctx.authority, crate::state::Authority::ChosenHuman) {
        return true;
    }
    if person_memory_subject_allowed(ctx, state, person).await {
        return true;
    }
    if has_preference_update {
        return false;
    }
    evidence_source_messages(ctx, state)
        .iter()
        .any(|message| message.person.as_ref() == Some(person))
        && restrictive_relationship_delta(
            trust_delta,
            familiarity_delta,
            valence_delta,
            closeness_delta,
            reliability_delta,
            reciprocity_delta,
            conflict_delta,
            proactive_consent,
        )
}

fn restrictive_relationship_delta(
    trust_delta: f32,
    familiarity_delta: f32,
    valence_delta: f32,
    closeness_delta: f32,
    reliability_delta: f32,
    reciprocity_delta: f32,
    conflict_delta: f32,
    proactive_consent: Option<&ProactiveConsent>,
) -> bool {
    trust_delta <= 0.0
        && familiarity_delta <= 0.0
        && valence_delta <= 0.0
        && closeness_delta <= 0.0
        && reliability_delta <= 0.0
        && reciprocity_delta <= 0.0
        && conflict_delta >= 0.0
        && proactive_consent.is_none_or(|consent| {
            matches!(
                consent,
                ProactiveConsent::Denied | ProactiveConsent::Unknown
            )
        })
}

fn review_relation_evidence(item: &Value, ctx: &SessionContext, state: &SessionState) -> Value {
    let supplied = item
        .get("evidence")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let mut evidence = json!({
        "action_id": ctx.action_id.0,
        "message_ids": relation_evidence_message_ids(item, ctx, state),
        "evidence": supplied,
    });
    if let Some(quote) = item["evidence_quote"]
        .as_str()
        .map(str::trim)
        .filter(|quote| !quote.is_empty())
    {
        evidence["quote"] = json!(quote);
    }
    evidence
}

fn relation_evidence_message_ids(
    item: &Value,
    ctx: &SessionContext,
    state: &SessionState,
) -> Vec<String> {
    let supplied = explicit_relation_evidence_message_ids(item);
    if !supplied.is_empty() {
        return supplied;
    }
    evidence_source_messages(ctx, state)
        .iter()
        .map(|message| message.message_id.clone())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>()
}

fn explicit_relation_evidence_message_ids(item: &Value) -> Vec<String> {
    let supplied = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
    if !supplied.is_empty() {
        return supplied;
    }
    if let Some(ids) = item
        .get("evidence")
        .and_then(|evidence| evidence.get("message_ids"))
        .map(|value| string_array(value).collect::<Vec<_>>())
        .filter(|ids| !ids.is_empty())
    {
        return ids;
    }
    vec![]
}

fn relation_asserted_by_person(
    item: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    source_kind: &RelationSource,
) -> Option<PersonId> {
    if let Some(person) = item["asserted_by_person_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| PersonId(id.to_string()))
    {
        return Some(person);
    }
    if !matches!(
        source_kind,
        RelationSource::Stated | RelationSource::ChosenHumanConfirmed
    ) {
        return None;
    }
    let evidence_ids = relation_evidence_message_ids(item, ctx, state);
    source_message_for_evidence(ctx, state, &evidence_ids).and_then(|message| message.person)
}
