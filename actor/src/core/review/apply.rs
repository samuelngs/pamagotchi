use crate::core::tools::{SessionContext, SessionState, permission, util};
use crate::identity::{
    PersonProfileLink, PersonProfileStatus, Relation, RelationDirection, RelationSource,
    RelationStatus, SocialRelation,
};
use crate::state::{
    Authority, BehaviorDirective, DirectiveScope, ProactiveConsent, RelationshipChange,
    RelationshipSignalUpdate,
};
use crate::store::{
    IntentRecord, Memory, MemoryKind, MemorySource, MemoryStability, MemorySubject,
    MemorySubjectType, MemoryType, MemoryUpdate, PrivacyCategory, ReviewOutputAudit, TruthStatus,
    VisibilityScope, memory_privacy_policy, memory_stability_policy, memory_truth_status_policy,
    sensitive_memory_next_review_at,
};
use protocol::{ConversationId, GroupId, InboundMessage, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};
use std::collections::HashSet;

const STRONG_LIKELY_PERSON_LINK_CONFIDENCE: f32 = 0.75;

#[derive(Default)]
struct ApplyCounts {
    profile_updates: usize,
    person_updates: usize,
    memories: usize,
    relationship_deltas: usize,
    social_relations: usize,
    directives: usize,
    open_loops: usize,
    conversation_summaries: usize,
    skipped: Vec<String>,
}

pub async fn apply(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    match ctx.store.review_outputs_for_action(&ctx.action_id.0).await {
        Ok(mut outputs) if !outputs.is_empty() => return already_applied_result(outputs.remove(0)),
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                action_id = %ctx.action_id.0,
                error = %e,
                "failed to check prior review output"
            );
        }
    }

    let source_action_id = review_source_action_id(ctx);
    if let Some(source_action_id) = source_action_id.as_deref() {
        match ctx
            .store
            .review_outputs_for_source_action(source_action_id)
            .await
        {
            Ok(mut outputs) if !outputs.is_empty() => {
                return already_applied_result(outputs.remove(0));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    action_id = %ctx.action_id.0,
                    source_action_id = %source_action_id,
                    error = %e,
                    "failed to check prior review output for source action"
                );
            }
        }
    }

    let mut counts = ApplyCounts::default();
    apply_profile_updates(args, ctx, state, &mut counts).await;
    apply_person_updates(args, ctx, state, &mut counts).await;
    apply_memories(args, ctx, state, &mut counts).await;
    apply_relationship_deltas(args, ctx, state, &mut counts).await;
    apply_social_relations(args, ctx, state, &mut counts).await;
    apply_directives(args, ctx, state, &mut counts).await;
    apply_open_loops(args, ctx, state, &mut counts).await;
    apply_conversation_summary(args, ctx, &mut counts).await;

    let mut result = review_result(&counts);
    let audit = ReviewOutputAudit {
        id: format!("review-output-{}", util::uuid_v4()),
        review_action_id: ctx.action_id.0.clone(),
        source_action_id,
        input: args.clone(),
        result: result.clone(),
        applied_at: util::now(),
    };
    match ctx.store.record_review_output(&audit).await {
        Ok(()) => {
            ctx.metrics
                .record_review_output(review_latency_ms(ctx, audit.applied_at).await);
        }
        Err(e) => {
            counts
                .skipped
                .push(format!("review output audit failed: {e}"));
            result = review_result(&counts);
        }
    }

    result.to_string()
}

fn already_applied_result(output: ReviewOutputAudit) -> String {
    let mut result = output.result;
    if let Some(result) = result.as_object_mut() {
        result.insert("status".into(), json!("already_applied"));
    }
    result.to_string()
}

async fn review_latency_ms(ctx: &SessionContext, applied_at: i64) -> Option<u64> {
    let source_action_id = review_source_action_id(ctx)?;
    let transcript = ctx.store.action_transcript(&source_action_id).await.ok()?;
    let ended_at = transcript.run?.ended_at?;
    Some(applied_at.saturating_sub(ended_at).max(0) as u64 * 1000)
}

fn review_result(counts: &ApplyCounts) -> Value {
    json!({
        "status": "applied",
        "profile_updates": counts.profile_updates,
        "person_updates": counts.person_updates,
        "memories": counts.memories,
        "relationship_deltas": counts.relationship_deltas,
        "social_relations": counts.social_relations,
        "directives": counts.directives,
        "open_loops": counts.open_loops,
        "conversation_summaries": counts.conversation_summaries,
        "skipped": counts.skipped,
    })
}

fn review_source_action_id(ctx: &SessionContext) -> Option<String> {
    ctx.cancelled_note
        .as_deref()
        .and_then(|note| note.strip_prefix("Post-turn review for action "))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

async fn apply_profile_updates(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    counts: &mut ApplyCounts,
) {
    for item in array_items(&args["profile_updates"]) {
        let Some(profile_id) = item["profile_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| ProfileId(id.to_string()))
        else {
            counts
                .skipped
                .push("profile_update missing profile_id".into());
            continue;
        };
        if !profile_update_target_allowed(ctx, state, item, &profile_id) {
            counts.skipped.push(format!(
                "profile {} is not present in review evidence",
                profile_id.0
            ));
            continue;
        }

        let display_name = item["display_name"]
            .as_str()
            .filter(|s| !s.trim().is_empty());
        let summary = item["summary"].as_str().filter(|s| !s.trim().is_empty());
        let comm_style = item["comm_style"].as_str().filter(|s| !s.trim().is_empty());
        if display_name.is_none() && summary.is_none() && comm_style.is_none() {
            counts
                .skipped
                .push(format!("profile {} had no fields", profile_id.0));
            continue;
        }

        let existing = ctx.store.get_profile(&profile_id).await.ok().flatten();
        let summary_update = summary.and_then(|summary| {
            merge_summary_update(
                existing
                    .as_ref()
                    .and_then(|profile| profile.summary.as_deref()),
                summary,
            )
        });

        let mut applied = false;
        if display_name.is_some() || summary_update.is_some() {
            if let Err(e) = ctx
                .store
                .update_profile(&profile_id, display_name, summary_update.as_deref())
                .await
            {
                counts
                    .skipped
                    .push(format!("profile {} update failed: {e}", profile_id.0));
                continue;
            }
            applied = true;
        }
        if let Some(comm_style) = comm_style {
            if let Err(e) = ctx
                .store
                .update_profile_comm_style(&profile_id, comm_style)
                .await
            {
                counts
                    .skipped
                    .push(format!("profile {} style failed: {e}", profile_id.0));
                continue;
            }
            applied = true;
        }
        if !applied {
            counts
                .skipped
                .push(format!("profile {} had no new fields", profile_id.0));
            continue;
        }
        counts.profile_updates += 1;
    }
}

async fn apply_person_updates(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    counts: &mut ApplyCounts,
) {
    for item in array_items(&args["person_updates"]) {
        let Some(person_id) = item["person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
        else {
            counts
                .skipped
                .push("person_update missing person_id".into());
            continue;
        };
        if !person_has_verified_or_strong_profile_context(ctx, &person_id).await {
            counts.skipped.push(format!(
                "person {} is not verified or strongly likely enough for person-level update",
                person_id.0
            ));
            continue;
        }
        if !person_update_target_allowed(ctx, state, item, &person_id) {
            counts.skipped.push(format!(
                "person {} is not present in review evidence",
                person_id.0
            ));
            continue;
        }

        let name = item["name"].as_str().filter(|s| !s.trim().is_empty());
        let summary = item["summary"].as_str().filter(|s| !s.trim().is_empty());
        let comm_style = item["comm_style"].as_str().filter(|s| !s.trim().is_empty());
        if name.is_none() && summary.is_none() && comm_style.is_none() {
            counts
                .skipped
                .push(format!("person {} had no fields", person_id.0));
            continue;
        }

        let existing = ctx.store.get_person(&person_id).await.ok().flatten();
        let summary_update = summary.and_then(|summary| {
            merge_summary_update(
                existing
                    .as_ref()
                    .and_then(|person| person.summary.as_deref()),
                summary,
            )
        });

        let mut applied = false;
        if name.is_some() || summary_update.is_some() {
            if let Err(e) = ctx
                .store
                .update_person(&person_id, name, summary_update.as_deref())
                .await
            {
                counts
                    .skipped
                    .push(format!("person {} update failed: {e}", person_id.0));
                continue;
            }
            applied = true;
        }
        if let Some(comm_style) = comm_style {
            if let Err(e) = ctx.store.update_comm_style(&person_id, comm_style).await {
                counts
                    .skipped
                    .push(format!("person {} style failed: {e}", person_id.0));
                continue;
            }
            applied = true;
        }
        if !applied {
            counts
                .skipped
                .push(format!("person {} had no new fields", person_id.0));
            continue;
        }
        counts.person_updates += 1;
    }
}

async fn apply_memories(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    for (idx, item) in array_items(&args["memories"]).enumerate() {
        let operation = item["operation"]
            .as_str()
            .map(str::trim)
            .filter(|operation| !operation.is_empty())
            .unwrap_or("upsert");
        match operation {
            "forget" => {
                apply_memory_forget(idx, item, ctx, state, counts).await;
                continue;
            }
            "reinforce" => {
                apply_memory_reinforce(idx, item, ctx, state, counts).await;
                continue;
            }
            "update" => {
                apply_memory_update(idx, item, ctx, state, counts).await;
                continue;
            }
            "contradict" | "mark_contradicted" => {
                apply_memory_contradict(idx, item, ctx, state, counts).await;
                continue;
            }
            "create" | "upsert" | "supersede" => {}
            _ => {
                counts
                    .skipped
                    .push(format!("memory {idx} has unsupported operation"));
                continue;
            }
        }
        let Some(content) = item["content"].as_str().filter(|s| !s.trim().is_empty()) else {
            counts.skipped.push("memory missing content".into());
            continue;
        };
        let supersede_target = if operation == "supersede" {
            let Some(target) = load_memory_supersede_target(idx, item, ctx, counts).await else {
                continue;
            };
            Some(target)
        } else {
            None
        };
        let evidence_message_ids = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
        if let Some(missing) = missing_evidence_message_ids(ctx, state, &evidence_message_ids) {
            counts.skipped.push(format!(
                "memory {idx} references unavailable evidence message ids: {}",
                missing.join(", ")
            ));
            continue;
        }
        let subjects = if operation == "supersede" && !item_has_key(item, "subjects") {
            supersede_target
                .as_ref()
                .map(|(_, memory)| memory.subjects.clone())
                .unwrap_or_default()
        } else {
            memory_subjects(item, ctx, state, counts, &evidence_message_ids).await
        };
        if !memory_profile_identity_subjects_allowed(ctx, state, item, &subjects) {
            counts.skipped.push(format!(
                "memory {idx} has profile or identity subject outside review evidence"
            ));
            continue;
        }
        if !memory_person_subjects_allowed(ctx, state, &subjects).await {
            counts.skipped.push(format!(
                "memory {idx} has unverified or weak person subject"
            ));
            continue;
        }
        if subjects.is_empty() {
            counts.skipped.push(format!("memory {idx} has no subject"));
            continue;
        }
        let supersedes = if operation == "supersede" {
            supersede_target
                .as_ref()
                .map(|(memory_id, _)| memory_id.clone())
        } else {
            item["supersedes"]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .map(|id| MemoryId(id.to_string()))
        };
        let reason = trimmed_text(item["reason"].as_str(), 512);
        let evidence = if operation == "supersede" {
            review_memory_operation_evidence(item, ctx, state, "supersede", reason.as_deref())
        } else {
            util::evidence_with_source_spans(
                item,
                json!({"source": "apply_review", "action_id": ctx.action_id.0}),
            )
        };
        let superseded_link_evidence = if operation == "supersede" {
            Some(review_memory_operation_evidence(
                item,
                ctx,
                state,
                "superseded",
                reason.as_deref(),
            ))
        } else {
            None
        };
        let supersede_key = supersedes.as_ref().map(|memory_id| {
            format!(
                "review:memory:supersede:{}:{}",
                memory_id.0,
                stable_hash(content)
            )
        });
        let dedupe_key = item["dedupe_key"]
            .as_str()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string)
            .or(supersede_key)
            .unwrap_or_else(|| match operation {
                "create" => {
                    format!(
                        "review:memory:create:{}:{}:{}",
                        ctx.action_id.0,
                        idx,
                        stable_hash(content)
                    )
                }
                _ => memory_upsert_dedupe_key(item, &subjects, content),
            });
        let apply_key = format!(
            "memory:{}:{}",
            dedupe_key,
            stable_hash(&format!("{}|{}", content, evidence_message_ids.join(",")))
        );
        if !state.applied_review_keys.insert(apply_key) {
            counts.skipped.push(format!("memory {idx} duplicate"));
            continue;
        }
        let now = util::now();
        let sensitivity = clamp_unit(item["sensitivity"].as_f64().unwrap_or(0.0) as f32);
        let sensitivity_category = item["sensitivity_category"].as_str().map(str::to_string);
        let explicit_privacy_category = item["privacy_category"]
            .as_str()
            .and_then(PrivacyCategory::parse);
        let explicit_visibility_scope = item["visibility_scope"]
            .as_str()
            .and_then(VisibilityScope::parse);
        let (privacy_category, visibility_scope) = memory_privacy_policy(
            sensitivity,
            sensitivity_category.as_deref(),
            explicit_privacy_category,
            explicit_visibility_scope,
        );
        let next_review_at = sensitive_memory_next_review_at(
            now,
            &privacy_category,
            item["next_review_at"].as_i64(),
        );
        let source_message = source_message_for_evidence(ctx, state, &evidence_message_ids);
        let source_conversation = item["conversation_id"]
            .as_str()
            .map(|id| ConversationId(id.to_string()))
            .or_else(|| {
                source_message
                    .as_ref()
                    .map(|message| message.conversation.clone())
            })
            .or_else(|| ctx.conversation.clone())
            .or_else(|| {
                ctx.messages
                    .first()
                    .map(|message| message.conversation.clone())
            });
        let embedding_result = ctx.router.embed_with_metadata(&[content]).await.ok();
        let embedding_model = embedding_result.as_ref().map(|result| result.model.clone());
        let embedding = embedding_result.and_then(|mut result| result.embeddings.pop());

        let memory_type = item["memory_type"]
            .as_str()
            .and_then(MemoryType::parse)
            .unwrap_or_default();
        let explicit_truth_status = item["truth_status"].as_str().and_then(TruthStatus::parse);
        let truth_status = memory_truth_status_policy(&memory_type, explicit_truth_status);
        let explicit_stability = item["stability"].as_str().and_then(MemoryStability::parse);
        let stability = memory_stability_policy(&memory_type, &truth_status, explicit_stability);

        let memory = Memory {
            id: MemoryId(format!("mem-{}", util::uuid_v4())),
            kind: item["kind"]
                .as_str()
                .and_then(MemoryKind::parse)
                .unwrap_or(MemoryKind::Semantic),
            memory_type,
            truth_status,
            content: content.to_string(),
            source: source_conversation
                .map(|conversation_id| MemorySource::Conversation {
                    conversation_id,
                    identity_id: source_message
                        .as_ref()
                        .and_then(|message| message.identity.clone()),
                    profile_id: source_message
                        .as_ref()
                        .and_then(|message| message.profile.clone()),
                    person_id: source_message
                        .as_ref()
                        .and_then(|message| message.person.clone()),
                    message_id: evidence_message_ids.first().cloned().or_else(|| {
                        source_message
                            .as_ref()
                            .map(|message| message.message_id.clone())
                    }),
                })
                .unwrap_or(MemorySource::Reflection),
            importance: clamp_unit(item["importance"].as_f64().unwrap_or(0.5) as f32),
            confidence: clamp_unit(item["confidence"].as_f64().unwrap_or(0.8) as f32),
            sensitivity,
            sensitivity_category,
            emotional_valence: clamp_valence(
                item["emotional_valence"].as_f64().unwrap_or(0.0) as f32
            ),
            created_at: now,
            accessed_at: now,
            access_count: 0,
            tags: string_array(&item["tags"]).collect(),
            subjects,
            evidence_message_ids,
            evidence_quote: item["evidence_quote"].as_str().map(str::to_string),
            evidence,
            expires_at: item["expires_at"].as_i64(),
            stability,
            supersedes,
            superseded_by: None,
            contradiction_group: item["contradiction_group"].as_str().map(str::to_string),
            privacy_category,
            visibility_scope,
            last_confirmed_at: item["last_confirmed_at"].as_i64(),
            next_review_at,
            dedupe_key: Some(dedupe_key),
            embedding_model,
            embedding_version: None,
            embedding,
        };

        match ctx.store.store_memory(&memory).await {
            Ok(id) => {
                if id == memory.id {
                    ctx.metrics.record_memory_created();
                } else {
                    ctx.metrics.record_memory_updated();
                }
                if let Some(superseded) = memory.supersedes.as_ref().filter(|old| *old != &id) {
                    if let Err(e) = ctx
                        .store
                        .update_memory(
                            superseded,
                            &MemoryUpdate {
                                truth_status: Some(TruthStatus::Outdated),
                                superseded_by: Some(id.clone()),
                                evidence: superseded_link_evidence.clone(),
                                ..Default::default()
                            },
                        )
                        .await
                    {
                        counts.skipped.push(format!(
                            "memory {idx} superseded link failed for {}: {e}",
                            superseded.0
                        ));
                    } else {
                        ctx.metrics.record_memory_updated();
                        ctx.metrics.record_memory_superseded();
                    }
                }
                state.memories_formed.push(id);
                counts.memories += 1;
            }
            Err(e) => counts.skipped.push(format!("memory {idx} failed: {e}")),
        }
    }
}

async fn load_memory_supersede_target(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    counts: &mut ApplyCounts,
) -> Option<(MemoryId, Memory)> {
    let Some(memory_id) = memory_target_id(item) else {
        counts
            .skipped
            .push(format!("memory {idx} supersede missing memory_id"));
        return None;
    };
    match ctx.store.get_memory(&memory_id).await {
        Ok(Some(memory)) => Some((memory_id, memory)),
        Ok(None) => {
            counts.skipped.push(format!(
                "memory {idx} supersede target {} not found",
                memory_id.0
            ));
            None
        }
        Err(e) => {
            counts
                .skipped
                .push(format!("memory {idx} supersede load failed: {e}"));
            None
        }
    }
}

async fn apply_memory_forget(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    let Some(memory_id) = item["memory_id"]
        .as_str()
        .or_else(|| item["id"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
    else {
        counts
            .skipped
            .push(format!("memory {idx} forget missing memory_id"));
        return;
    };
    let reason = trimmed_text(item["reason"].as_str(), 512)
        .or_else(|| trimmed_text(item["noise_reason"].as_str(), 512))
        .unwrap_or_else(|| "review classified memory as noise".into());
    let apply_key = format!(
        "memory:forget:{}:{}",
        memory_id.0,
        stable_hash(reason.as_str())
    );
    if !state.applied_review_keys.insert(apply_key) {
        counts
            .skipped
            .push(format!("memory {idx} forget duplicate"));
        return;
    }
    match ctx
        .store
        .forget_with_reason(&memory_id, Some(reason.as_str()))
        .await
    {
        Ok(true) => counts.memories += 1,
        Ok(false) => counts.skipped.push(format!(
            "memory {idx} forget target {} not found",
            memory_id.0
        )),
        Err(e) => counts
            .skipped
            .push(format!("memory {idx} forget failed: {e}")),
    }
}

async fn apply_memory_reinforce(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    let Some((memory_id, existing)) = load_memory_target(idx, item, ctx, counts, "reinforce").await
    else {
        return;
    };
    if !reserve_memory_operation(idx, "reinforce", item, &memory_id, state, counts) {
        return;
    }
    let new_evidence_ids = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
    if let Some(missing) = missing_evidence_message_ids(ctx, state, &new_evidence_ids) {
        counts.skipped.push(format!(
            "memory {idx} reinforce references unavailable evidence message ids: {}",
            missing.join(", ")
        ));
        return;
    }

    let reason = trimmed_text(item["reason"].as_str(), 512);
    let confidence = item["confidence"]
        .as_f64()
        .map(|value| clamp_unit(value as f32))
        .unwrap_or_else(|| (existing.confidence + 0.05).min(1.0))
        .max(existing.confidence);
    let mut update = MemoryUpdate {
        confidence: Some(confidence),
        last_confirmed_at: Some(item["last_confirmed_at"].as_i64().unwrap_or_else(util::now)),
        evidence: Some(review_memory_operation_evidence(
            item,
            ctx,
            state,
            "reinforce",
            reason.as_deref(),
        )),
        ..Default::default()
    };
    if let Some(importance) = item["importance"].as_f64() {
        update.importance = Some(existing.importance.max(clamp_unit(importance as f32)));
    }
    if !new_evidence_ids.is_empty() {
        update.evidence_message_ids = Some(merge_string_lists(
            existing.evidence_message_ids,
            new_evidence_ids,
        ));
    }
    if let Some(quote) = trimmed_text(item["evidence_quote"].as_str(), 512) {
        update.evidence_quote = Some(quote);
    }
    if let Some(status) = item["truth_status"].as_str().and_then(TruthStatus::parse) {
        update.truth_status = Some(status);
    }

    apply_existing_memory_update(idx, "reinforce", &memory_id, update, ctx, counts).await;
}

async fn apply_memory_update(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    let Some((memory_id, _existing)) = load_memory_target(idx, item, ctx, counts, "update").await
    else {
        return;
    };
    if !reserve_memory_operation(idx, "update", item, &memory_id, state, counts) {
        return;
    }
    let reason = trimmed_text(item["reason"].as_str(), 512);
    let Some(update) =
        memory_update_from_item(idx, item, ctx, state, counts, "update", reason.as_deref()).await
    else {
        return;
    };
    apply_existing_memory_update(idx, "update", &memory_id, update, ctx, counts).await;
}

async fn apply_memory_contradict(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    let Some((memory_id, existing)) =
        load_memory_target(idx, item, ctx, counts, "contradict").await
    else {
        return;
    };
    if !reserve_memory_operation(idx, "contradict", item, &memory_id, state, counts) {
        return;
    }
    let evidence_ids = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
    if let Some(missing) = missing_evidence_message_ids(ctx, state, &evidence_ids) {
        counts.skipped.push(format!(
            "memory {idx} contradict references unavailable evidence message ids: {}",
            missing.join(", ")
        ));
        return;
    }

    let reason = trimmed_text(item["reason"].as_str(), 512)
        .unwrap_or_else(|| "review found contradictory evidence".into());
    let mut update = MemoryUpdate {
        truth_status: Some(
            item["truth_status"]
                .as_str()
                .and_then(TruthStatus::parse)
                .unwrap_or(TruthStatus::Denied),
        ),
        contradiction_group: Some(
            trimmed_text(item["contradiction_group"].as_str(), 128).unwrap_or_else(|| {
                format!(
                    "contradiction:{}:{}",
                    memory_id.0,
                    stable_hash(reason.as_str())
                )
            }),
        ),
        evidence: Some(review_memory_operation_evidence(
            item,
            ctx,
            state,
            "contradict",
            Some(reason.as_str()),
        )),
        ..Default::default()
    };
    if let Some(confidence) = item["confidence"].as_f64() {
        update.confidence = Some(clamp_unit(confidence as f32).min(existing.confidence));
    }
    if !evidence_ids.is_empty() {
        update.evidence_message_ids = Some(merge_string_lists(
            existing.evidence_message_ids,
            evidence_ids,
        ));
    }
    if let Some(quote) = trimmed_text(item["evidence_quote"].as_str(), 512) {
        update.evidence_quote = Some(quote);
    }
    if let Some(superseded_by) = item["superseded_by"]
        .as_str()
        .or_else(|| item["superseding_memory_id"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
    {
        update.superseded_by = Some(superseded_by);
    }

    apply_existing_memory_update(idx, "contradict", &memory_id, update, ctx, counts).await;
}

async fn load_memory_target(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    counts: &mut ApplyCounts,
    operation: &str,
) -> Option<(MemoryId, Memory)> {
    let Some(memory_id) = memory_target_id(item) else {
        counts
            .skipped
            .push(format!("memory {idx} {operation} missing memory_id"));
        return None;
    };
    match ctx.store.get_memory(&memory_id).await {
        Ok(Some(memory)) => Some((memory_id, memory)),
        Ok(None) => {
            counts.skipped.push(format!(
                "memory {idx} {operation} target {} not found",
                memory_id.0
            ));
            None
        }
        Err(e) => {
            counts
                .skipped
                .push(format!("memory {idx} {operation} load failed: {e}"));
            None
        }
    }
}

fn memory_target_id(item: &Value) -> Option<MemoryId> {
    item["memory_id"]
        .as_str()
        .or_else(|| item["id"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
}

fn reserve_memory_operation(
    idx: usize,
    operation: &str,
    item: &Value,
    memory_id: &MemoryId,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) -> bool {
    let payload = serde_json::to_string(item).unwrap_or_default();
    let apply_key = format!(
        "memory:{operation}:{}:{}",
        memory_id.0,
        stable_hash(payload.as_str())
    );
    if state.applied_review_keys.insert(apply_key) {
        true
    } else {
        counts
            .skipped
            .push(format!("memory {idx} {operation} duplicate"));
        false
    }
}

async fn memory_update_from_item(
    idx: usize,
    item: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    counts: &mut ApplyCounts,
    operation: &str,
    reason: Option<&str>,
) -> Option<MemoryUpdate> {
    let mut update = MemoryUpdate::default();
    if let Some(content) = trimmed_text(item["content"].as_str(), 4096) {
        let embedding_result = ctx
            .router
            .embed_with_metadata(&[content.as_str()])
            .await
            .ok();
        update.embedding_model = embedding_result.as_ref().map(|result| result.model.clone());
        update.embedding = embedding_result.and_then(|mut result| result.embeddings.pop());
        update.content = Some(content);
    }
    if let Some(memory_type) = item["memory_type"].as_str().and_then(MemoryType::parse) {
        update.memory_type = Some(memory_type);
    }
    if let Some(truth_status) = item["truth_status"].as_str().and_then(TruthStatus::parse) {
        update.truth_status = Some(truth_status);
    }
    if let Some(importance) = item["importance"].as_f64() {
        update.importance = Some(clamp_unit(importance as f32));
    }
    if let Some(confidence) = item["confidence"].as_f64() {
        update.confidence = Some(clamp_unit(confidence as f32));
    }
    if let Some(sensitivity) = item["sensitivity"].as_f64() {
        update.sensitivity = Some(clamp_unit(sensitivity as f32));
    }
    if let Some(category) = trimmed_text(item["sensitivity_category"].as_str(), 128) {
        update.sensitivity_category = Some(category);
    }
    if let Some(valence) = item["emotional_valence"].as_f64() {
        update.emotional_valence = Some(clamp_valence(valence as f32));
    }
    if item_has_key(item, "tags") {
        update.tags = Some(string_array(&item["tags"]).collect());
    }
    if item_has_key(item, "subjects") {
        let subjects = explicit_memory_subjects(&item["subjects"], counts);
        if subjects.is_empty() {
            counts
                .skipped
                .push(format!("memory {idx} {operation} has no valid subjects"));
            return None;
        }
        if !memory_person_subjects_allowed(ctx, state, &subjects).await {
            counts.skipped.push(format!(
                "memory {idx} {operation} has unverified or weak person subject"
            ));
            return None;
        }
        if !memory_profile_identity_subjects_allowed(ctx, state, item, &subjects) {
            counts.skipped.push(format!(
                "memory {idx} {operation} has profile or identity subject outside review evidence"
            ));
            return None;
        }
        update.subjects = Some(subjects);
    }
    if item_has_key(item, "evidence_message_ids") {
        let evidence_message_ids = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
        if let Some(missing) = missing_evidence_message_ids(ctx, state, &evidence_message_ids) {
            counts.skipped.push(format!(
                "memory {idx} {operation} references unavailable evidence message ids: {}",
                missing.join(", ")
            ));
            return None;
        }
        update.evidence_message_ids = Some(evidence_message_ids);
    }
    if let Some(quote) = trimmed_text(item["evidence_quote"].as_str(), 512) {
        update.evidence_quote = Some(quote);
    }
    if memory_operation_updates_evidence(item, reason) {
        update.evidence = Some(review_memory_operation_evidence(
            item, ctx, state, operation, reason,
        ));
    }
    if let Some(expires_at) = item["expires_at"].as_i64() {
        update.expires_at = Some(expires_at);
    }
    if let Some(stability) = item["stability"].as_str().and_then(MemoryStability::parse) {
        update.stability = Some(stability);
    }
    if let Some(supersedes) = item["supersedes"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
    {
        update.supersedes = Some(supersedes);
    }
    if let Some(superseded_by) = item["superseded_by"]
        .as_str()
        .or_else(|| item["superseding_memory_id"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
    {
        update.superseded_by = Some(superseded_by);
    }
    if let Some(group) = trimmed_text(item["contradiction_group"].as_str(), 128) {
        update.contradiction_group = Some(group);
    }
    if let Some(privacy) = item["privacy_category"]
        .as_str()
        .and_then(PrivacyCategory::parse)
    {
        update.privacy_category = Some(privacy);
    }
    if let Some(scope) = item["visibility_scope"]
        .as_str()
        .and_then(VisibilityScope::parse)
    {
        update.visibility_scope = Some(scope);
    }
    if let Some(last_confirmed_at) = item["last_confirmed_at"].as_i64() {
        update.last_confirmed_at = Some(last_confirmed_at);
    }
    if let Some(next_review_at) = item["next_review_at"].as_i64() {
        update.next_review_at = Some(next_review_at);
    }
    if let Some(dedupe_key) = trimmed_text(item["dedupe_key"].as_str(), 256) {
        update.dedupe_key = Some(dedupe_key);
    }

    Some(update)
}

async fn apply_existing_memory_update(
    idx: usize,
    operation: &str,
    memory_id: &MemoryId,
    update: MemoryUpdate,
    ctx: &SessionContext,
    counts: &mut ApplyCounts,
) {
    if !memory_update_has_fields(&update) {
        counts
            .skipped
            .push(format!("memory {idx} {operation} had no update fields"));
        return;
    }
    let superseded = update.superseded_by.is_some();
    match ctx.store.update_memory(memory_id, &update).await {
        Ok(()) => {
            ctx.metrics.record_memory_updated();
            if superseded {
                ctx.metrics.record_memory_superseded();
            }
            counts.memories += 1;
        }
        Err(e) => counts
            .skipped
            .push(format!("memory {idx} {operation} failed: {e}")),
    }
}

async fn apply_relationship_deltas(
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

async fn apply_social_relations(
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
        if matches!(source_kind, RelationSource::ChosenPersonConfirmed)
            && !matches!(ctx.authority, crate::state::Authority::ChosenPerson)
        {
            counts.skipped.push(format!(
                "social_relation {idx} chosen-person confirmation denied"
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

async fn apply_directives(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    let mut existing_ids = match ctx.store.list_directives().await {
        Ok(directives) => directives
            .into_iter()
            .map(|directive| directive.id)
            .collect::<HashSet<_>>(),
        Err(e) => {
            if args["directives"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                counts.skipped.push(format!(
                    "directives could not load existing directives: {e}"
                ));
            }
            return;
        }
    };

    for (idx, item) in array_items(&args["directives"]).enumerate() {
        let Some(text) = trimmed_text(item["directive"].as_str(), 600) else {
            counts.skipped.push(format!("directive {idx} missing text"));
            continue;
        };
        let Some(scope) = directive_scope(item, ctx).await else {
            counts
                .skipped
                .push(format!("directive {idx} has unsupported or missing scope"));
            continue;
        };
        if !directive_scope_allowed(&scope, ctx).await {
            counts.skipped.push(format!(
                "directive {idx} targets a scope outside review context"
            ));
            continue;
        }
        let Some(set_by) = directive_set_by(item, ctx) else {
            counts.skipped.push(format!(
                "directive {idx} has no current person to attribute"
            ));
            continue;
        };

        let id = trimmed_text(item["id"].as_str(), 128)
            .or_else(|| trimmed_text(item["dedupe_key"].as_str(), 128))
            .unwrap_or_else(|| directive_id(&scope, &text));
        if !state.applied_review_keys.insert(format!("directive:{id}")) {
            counts.skipped.push(format!("directive {idx} duplicate"));
            continue;
        }
        if existing_ids.contains(&id) {
            counts
                .skipped
                .push(format!("directive {idx} already exists"));
            continue;
        }

        let priority = item["priority"].as_i64().unwrap_or(0).clamp(-100, 100) as i32;
        let directive = BehaviorDirective {
            id: id.clone(),
            scope,
            directive: text,
            set_by,
            priority,
            active: item["active"].as_bool().unwrap_or(true),
            created_at: util::now(),
            expires_at: item["expires_at"].as_i64(),
        };
        match ctx.store.add_directive(&directive).await {
            Ok(()) => {
                existing_ids.insert(id);
                counts.directives += 1;
            }
            Err(e) => counts.skipped.push(format!("directive {idx} failed: {e}")),
        }
    }
}

async fn directive_scope(item: &Value, ctx: &SessionContext) -> Option<DirectiveScope> {
    match item["scope"].as_str()? {
        "global" => Some(DirectiveScope::Global),
        "authority" => item["authority"]
            .as_str()
            .and_then(Authority::parse)
            .map(DirectiveScope::Authority),
        "person" => item["person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| DirectiveScope::Person(PersonId(id.to_string())))
            .or_else(|| current_review_person(ctx).map(DirectiveScope::Person)),
        "group" => {
            if let Some(id) = item["group_id"].as_str().filter(|id| !id.trim().is_empty()) {
                Some(DirectiveScope::Group(GroupId(id.to_string())))
            } else {
                current_review_group(ctx).await.map(DirectiveScope::Group)
            }
        }
        _ => None,
    }
}

async fn directive_scope_allowed(scope: &DirectiveScope, ctx: &SessionContext) -> bool {
    if matches!(ctx.authority, Authority::ChosenPerson) {
        return true;
    }

    match scope {
        DirectiveScope::Person(person) => current_review_person(ctx).as_ref() == Some(person),
        DirectiveScope::Group(group) => current_review_group(ctx).await.as_ref() == Some(group),
        DirectiveScope::Global | DirectiveScope::Authority(_) => false,
    }
}

fn directive_set_by(item: &Value, ctx: &SessionContext) -> Option<PersonId> {
    if matches!(ctx.authority, Authority::ChosenPerson) {
        return item["set_by_person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
            .or_else(|| current_review_person(ctx));
    }
    current_review_person(ctx)
}

fn current_review_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.messages
        .iter()
        .find_map(|message| message.person.clone())
}

async fn current_review_group(ctx: &SessionContext) -> Option<GroupId> {
    if let Some(group) = ctx
        .messages
        .iter()
        .find_map(|message| message.group.clone())
    {
        return Some(group);
    }
    let conversation = ctx.conversation.as_ref()?;
    ctx.store
        .list_conversations()
        .await
        .ok()?
        .into_iter()
        .find(|summary| summary.id == *conversation)
        .and_then(|summary| summary.group)
}

fn directive_id(scope: &DirectiveScope, directive: &str) -> String {
    let scope_type = scope.scope_type();
    let scope_value = scope.scope_value().unwrap_or_default();
    format!(
        "directive-{}",
        stable_hash(&format!("{scope_type}:{scope_value}:{directive}"))
    )
}

async fn apply_open_loops(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    for (idx, item) in array_items(&args["open_loops"]).enumerate() {
        let Some(task) = item["task"].as_str().filter(|task| !task.trim().is_empty()) else {
            counts.skipped.push("open_loop missing task".into());
            continue;
        };
        let fire_at = item["fire_at"].as_i64();
        let condition = item["condition"]
            .as_str()
            .map(str::trim)
            .filter(|condition| !condition.is_empty())
            .map(str::to_string);
        let Some(kind) =
            normalize_open_loop_kind(item["kind"].as_str(), fire_at, condition.as_deref())
        else {
            counts
                .skipped
                .push(format!("open_loop {idx} has unsupported kind"));
            continue;
        };
        if kind == "scheduled" && fire_at.is_none() {
            counts
                .skipped
                .push(format!("open_loop {idx} missing fire_at"));
            continue;
        }
        if kind == "triggered" && condition.is_none() {
            counts
                .skipped
                .push(format!("open_loop {idx} missing condition"));
            continue;
        }
        if permission::intent_requires_chosen_person_approval(item)
            && !matches!(ctx.authority, crate::state::Authority::ChosenPerson)
        {
            match create_chosen_person_proactive_approval_intent(
                item,
                ctx,
                idx,
                task,
                kind,
                fire_at,
                condition.as_deref(),
            )
            .await
            {
                Some(_) => counts.open_loops += 1,
                None => counts.skipped.push(format!(
                    "open_loop {idx} requires chosen-person approval for sensitive proactive outreach"
                )),
            }
            continue;
        }
        if !matches!(ctx.authority, crate::state::Authority::ChosenPerson) {
            match permission::intent_targets_current_or_verified_with_keys(
                item,
                ctx,
                "person_id",
                "profile_id",
                "conversation_id",
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => {
                    counts.skipped.push(format!(
                        "open_loop {idx} targets an unverified third-party outreach"
                    ));
                    continue;
                }
                Err(e) => {
                    counts
                        .skipped
                        .push(format!("open_loop {idx} target verification failed: {e}"));
                    continue;
                }
            }
        }
        let now = util::now();
        let conversation = item["conversation_id"]
            .as_str()
            .map(|id| ConversationId(id.to_string()))
            .or_else(|| ctx.conversation.clone());
        let person = item["person_id"]
            .as_str()
            .map(|id| PersonId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.person.clone())
            });
        let profile = item["profile_id"]
            .as_str()
            .map(|id| ProfileId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.profile.clone())
            });
        let dedupe_key = item["dedupe_key"]
            .as_str()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "review:{}:open_loop:{}:{}:{}:{}",
                    ctx.action_id.0,
                    idx,
                    open_loop_timing_key(fire_at, condition.as_deref()),
                    conversation
                        .as_ref()
                        .map(|conversation| conversation.0.as_str())
                        .unwrap_or("none"),
                    stable_hash(task)
                )
            });
        if !state
            .applied_review_keys
            .insert(format!("open_loop:{dedupe_key}"))
        {
            counts.skipped.push(format!("open_loop {idx} duplicate"));
            continue;
        }
        let intent = IntentRecord {
            id: format!("intent-{}", util::uuid_v4()),
            kind: kind.to_string(),
            status: "active".into(),
            task: task.to_string(),
            person,
            profile,
            conversation: conversation.clone(),
            fire_at: if kind == "scheduled" { fire_at } else { None },
            condition: if kind == "triggered" { condition } else { None },
            recurrence: None,
            priority: item["priority"].as_u64().unwrap_or(50).min(100) as u8,
            dedupe_key: Some(dedupe_key),
            source_action: Some(ctx.action_id.0.clone()),
            source_memory: source_memory_id(item),
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_person_approved: matches!(ctx.authority, crate::state::Authority::ChosenPerson),
        };
        match ctx.store.create_intent(&intent).await {
            Ok(()) => counts.open_loops += 1,
            Err(e) => counts.skipped.push(format!("open_loop {idx} failed: {e}")),
        }
    }
}

fn source_memory_id(item: &Value) -> Option<MemoryId> {
    item["source_memory_id"]
        .as_str()
        .or_else(|| item["source_memory"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
}

fn open_loop_timing_key(fire_at: Option<i64>, condition: Option<&str>) -> String {
    if let Some(fire_at) = fire_at {
        return fire_at.to_string();
    }
    condition
        .map(stable_hash)
        .unwrap_or_else(|| "unspecified".into())
}

fn normalize_open_loop_kind(
    kind: Option<&str>,
    fire_at: Option<i64>,
    condition: Option<&str>,
) -> Option<&'static str> {
    match kind.map(str::trim).filter(|kind| !kind.is_empty()) {
        Some("scheduled") => Some("scheduled"),
        Some("triggered") => Some("triggered"),
        Some("follow_up") | None => {
            if condition.is_some() && fire_at.is_none() {
                Some("triggered")
            } else {
                Some("scheduled")
            }
        }
        Some(_) => None,
    }
}

async fn create_chosen_person_proactive_approval_intent(
    item: &Value,
    ctx: &SessionContext,
    idx: usize,
    task: &str,
    kind: &str,
    fire_at: Option<i64>,
    condition: Option<&str>,
) -> Option<String> {
    let chosen_person = chosen_person(ctx)?;
    let now = util::now();
    let original_dedupe_key = item["dedupe_key"]
        .as_str()
        .filter(|key| !key.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "review:{}:open_loop:{}:{}:{}",
                ctx.action_id.0,
                idx,
                open_loop_timing_key(fire_at, condition),
                stable_hash(task)
            )
        });
    let pending_id = format!("intent-{}", util::uuid_v4());
    let pending_intent = IntentRecord {
        id: pending_id.clone(),
        kind: kind.to_string(),
        status: "pending_approval".into(),
        task: task.to_string(),
        person: item["person_id"]
            .as_str()
            .map(|id| PersonId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.person.clone())
            }),
        profile: item["profile_id"]
            .as_str()
            .map(|id| ProfileId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.profile.clone())
            }),
        conversation: item["conversation_id"]
            .as_str()
            .map(|id| ConversationId(id.to_string()))
            .or_else(|| ctx.conversation.clone()),
        fire_at: if kind == "scheduled" { fire_at } else { None },
        condition: if kind == "triggered" {
            condition.map(str::to_string)
        } else {
            None
        },
        recurrence: item["recurrence"].as_str().map(str::to_string),
        priority: item["priority"].as_u64().unwrap_or(50).min(100) as u8,
        dedupe_key: Some(original_dedupe_key.clone()),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: source_memory_id(item),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_person_approved: false,
    };
    if let Err(e) = ctx.store.create_intent(&pending_intent).await {
        tracing::warn!(
            action = %ctx.action_id,
            %e,
            "failed to create pending sensitive open loop intent"
        );
        return None;
    }
    let target = chosen_person_approval_target_description(item, ctx, fire_at, condition);
    let intent = IntentRecord {
        id: format!("intent-{}", util::uuid_v4()),
        kind: "scheduled".into(),
        status: "active".into(),
        task: format!(
            "Review sensitive proactive outreach before it is sent. Pending intent: {pending_id}. Proposed task: {task}. {target} If the chosen person approves, update intent {pending_id} with status active. If the chosen person declines, delete intent {pending_id}."
        ),
        person: Some(chosen_person),
        profile: None,
        conversation: None,
        fire_at: Some(now),
        condition: None,
        recurrence: None,
        priority: 100,
        dedupe_key: Some(format!(
            "chosen-person-approval:sensitive-open-loop:{original_dedupe_key}"
        )),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: source_memory_id(item),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_person_approved: true,
    };
    let id = intent.id.clone();
    match ctx.store.create_intent(&intent).await {
        Ok(()) => Some(id),
        Err(e) => {
            tracing::warn!(
                action = %ctx.action_id,
                %e,
                "failed to create chosen-person approval intent for sensitive open loop"
            );
            None
        }
    }
}

fn chosen_person(ctx: &SessionContext) -> Option<PersonId> {
    let actor = ctx.state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, relationship)| {
            matches!(
                relationship.authority,
                crate::state::Authority::ChosenPerson
            )
        })
        .map(|(person, _)| person.clone())
}

fn chosen_person_approval_target_description(
    item: &Value,
    ctx: &SessionContext,
    fire_at: Option<i64>,
    condition: Option<&str>,
) -> String {
    let person = item["person_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            ctx.messages
                .first()
                .and_then(|message| message.person.as_ref())
                .map(|id| id.0.clone())
        })
        .unwrap_or_else(|| "unknown person".into());
    let profile = item["profile_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            ctx.messages
                .first()
                .and_then(|message| message.profile.as_ref())
                .map(|id| id.0.clone())
        })
        .unwrap_or_else(|| "unknown profile".into());
    let conversation = item["conversation_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| ctx.conversation.as_ref().map(|id| id.0.clone()))
        .unwrap_or_else(|| "unknown conversation".into());
    let timing = if let Some(fire_at) = fire_at {
        format!("Proposed fire_at: {fire_at}.")
    } else if let Some(condition) = condition {
        format!("Proposed condition: {condition}.")
    } else {
        "No timing specified.".into()
    };
    format!(
        "Target person: {person}. Target profile: {profile}. Target conversation: {conversation}. {timing}"
    )
}

async fn apply_conversation_summary(args: &Value, ctx: &SessionContext, counts: &mut ApplyCounts) {
    let summary_obj = &args["conversation_summary"];
    if !summary_obj.is_object() {
        return;
    }
    let Some(summary) = summary_obj["summary"]
        .as_str()
        .filter(|summary| !summary.trim().is_empty())
    else {
        counts
            .skipped
            .push("conversation_summary missing summary".into());
        return;
    };
    let Some(conversation) = summary_obj["conversation_id"]
        .as_str()
        .map(|id| ConversationId(id.to_string()))
        .or_else(|| ctx.conversation.clone())
    else {
        counts
            .skipped
            .push("conversation_summary missing conversation".into());
        return;
    };
    let covered = string_array(&summary_obj["covered_message_ids"]).collect::<Vec<_>>();
    let existing = ctx
        .store
        .list_conversations()
        .await
        .ok()
        .and_then(|conversations| {
            conversations
                .into_iter()
                .find(|candidate| candidate.id == conversation)
        });
    let summary_update = merge_summary_update(
        existing
            .as_ref()
            .and_then(|conversation| conversation.summary.as_deref()),
        summary,
    );
    let existing_covered = existing
        .as_ref()
        .map(|conversation| conversation.summary_covered_message_ids.clone())
        .unwrap_or_default();
    let merged_covered = merge_ordered_ids(existing_covered.clone(), covered);
    let covered_changed = merged_covered != existing_covered;
    let summary_to_store = summary_update.or_else(|| {
        covered_changed.then(|| {
            existing
                .as_ref()
                .and_then(|conversation| conversation.summary.clone())
                .unwrap_or_else(|| summary.to_string())
        })
    });
    let Some(summary_to_store) = summary_to_store else {
        counts.skipped.push(format!(
            "conversation_summary {} had no new fields",
            conversation.0
        ));
        return;
    };
    match ctx
        .store
        .update_conversation_summary(&conversation, &summary_to_store, &merged_covered)
        .await
    {
        Ok(()) => counts.conversation_summaries += 1,
        Err(e) => counts
            .skipped
            .push(format!("conversation_summary failed: {e}")),
    }
}

fn profile_update_target_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    item: &Value,
    profile: &ProfileId,
) -> bool {
    matches!(ctx.authority, crate::state::Authority::ChosenPerson)
        || evidence_message_matches_target(item, ctx, state, |message| {
            message.profile.as_ref() == Some(profile)
        })
}

fn person_update_target_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    item: &Value,
    person: &PersonId,
) -> bool {
    matches!(ctx.authority, crate::state::Authority::ChosenPerson)
        || evidence_message_matches_target(item, ctx, state, |message| {
            message.person.as_ref() == Some(person)
        })
}

fn evidence_message_matches_target(
    item: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    mut matches_target: impl FnMut(&InboundMessage) -> bool,
) -> bool {
    let evidence_ids = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
    let messages = evidence_source_messages(ctx, state);
    if evidence_ids.is_empty() {
        return messages.iter().any(matches_target);
    }
    messages.iter().any(|message| {
        matches_target(message)
            && evidence_ids
                .iter()
                .any(|id| id.as_str() == message.message_id)
    })
}

async fn memory_subjects(
    item: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    counts: &mut ApplyCounts,
    evidence_message_ids: &[String],
) -> Vec<MemorySubject> {
    let mut subjects = explicit_memory_subjects(&item["subjects"], counts);

    if subjects.is_empty() {
        let source_message = source_message_for_evidence(ctx, state, evidence_message_ids);
        if let Some(profile) = source_message
            .as_ref()
            .and_then(|message| message.profile.clone())
        {
            subjects.push(MemorySubject::profile(profile, Some("about".into()), 1.0));
        } else if let Some(identity) = source_message
            .as_ref()
            .and_then(|message| message.identity.clone())
        {
            subjects.push(MemorySubject::identity(identity, Some("about".into()), 1.0));
        }
    }
    subjects
}

fn explicit_memory_subjects(value: &Value, counts: &mut ApplyCounts) -> Vec<MemorySubject> {
    let mut subjects = Vec::new();
    for subject in array_items(value) {
        let Some(subject_type) = subject["type"].as_str().and_then(MemorySubjectType::parse) else {
            counts.skipped.push("memory subject missing type".into());
            continue;
        };
        let Some(subject_id) = subject["id"].as_str().filter(|id| !id.trim().is_empty()) else {
            counts.skipped.push("memory subject missing id".into());
            continue;
        };
        let role = subject["role"].as_str().map(str::to_string);
        let confidence = clamp_unit(subject["confidence"].as_f64().unwrap_or(1.0) as f32);
        subjects.push(MemorySubject {
            subject_type,
            subject_id: subject_id.to_string(),
            role,
            confidence,
        });
    }
    subjects
}

fn memory_profile_identity_subjects_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    item: &Value,
    subjects: &[MemorySubject],
) -> bool {
    if matches!(ctx.authority, Authority::ChosenPerson) || !item_has_key(item, "subjects") {
        return true;
    }

    subjects.iter().all(|subject| match subject.subject_type {
        MemorySubjectType::Profile => {
            evidence_message_matches_target(item, ctx, state, |message| {
                message
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile.0.as_str() == subject.subject_id.as_str())
            })
        }
        MemorySubjectType::Identity => {
            evidence_message_matches_target(item, ctx, state, |message| {
                message
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.0.as_str() == subject.subject_id.as_str())
            })
        }
        MemorySubjectType::Actor | MemorySubjectType::Person => true,
    })
}

async fn memory_person_subjects_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    subjects: &[MemorySubject],
) -> bool {
    for subject in subjects
        .iter()
        .filter(|subject| subject.subject_type == MemorySubjectType::Person)
    {
        if !person_memory_subject_allowed(ctx, state, &PersonId(subject.subject_id.clone())).await {
            return false;
        }
    }
    true
}

async fn person_has_verified_or_strong_profile_context(
    ctx: &SessionContext,
    person: &PersonId,
) -> bool {
    let Ok(profiles) = ctx.store.get_profiles_for_person(person).await else {
        return false;
    };
    profiles
        .into_iter()
        .any(|(_, link)| person_link_allows_person_level_update(&link))
}

fn person_link_allows_person_level_update(link: &PersonProfileLink) -> bool {
    match link.status {
        PersonProfileStatus::Verified => true,
        PersonProfileStatus::Likely => link.confidence >= STRONG_LIKELY_PERSON_LINK_CONFIDENCE,
        _ => false,
    }
}

async fn person_memory_subject_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    person: &PersonId,
) -> bool {
    evidence_source_messages(ctx, state)
        .iter()
        .any(|message| message.person.as_ref() == Some(person))
        && person_has_verified_or_strong_profile_context(ctx, person).await
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
    if matches!(ctx.authority, crate::state::Authority::ChosenPerson) {
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
        RelationSource::Stated | RelationSource::ChosenPersonConfirmed
    ) {
        return None;
    }
    let evidence_ids = relation_evidence_message_ids(item, ctx, state);
    source_message_for_evidence(ctx, state, &evidence_ids).and_then(|message| message.person)
}

fn memory_upsert_dedupe_key(item: &Value, subjects: &[MemorySubject], content: &str) -> String {
    let mut subject_keys = subjects
        .iter()
        .map(|subject| {
            format!(
                "{}:{}:{}",
                subject.subject_type.as_str(),
                subject.subject_id,
                subject.role.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    subject_keys.sort();
    let subject_hash = stable_hash(&subject_keys.join("|"));
    let kind = item["kind"].as_str().unwrap_or("semantic");
    let memory_type = item["memory_type"].as_str().unwrap_or("general");
    let truth_status = item["truth_status"].as_str().unwrap_or("stated");
    format!(
        "review:memory:upsert:{kind}:{memory_type}:{truth_status}:{subject_hash}:{}",
        stable_hash(content)
    )
}

fn review_memory_operation_evidence(
    item: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    operation: &str,
    reason: Option<&str>,
) -> Value {
    let mut evidence = util::evidence_with_source_spans(item, json!({}));
    if !evidence.is_object() {
        evidence = json!({ "evidence": evidence });
    }
    let object = evidence.as_object_mut().expect("evidence object");
    object
        .entry("source")
        .or_insert_with(|| json!("apply_review"));
    object
        .entry("action_id")
        .or_insert_with(|| json!(ctx.action_id.0.clone()));
    object
        .entry("operation")
        .or_insert_with(|| json!(operation));
    if let Some(reason) = reason {
        object.entry("reason").or_insert_with(|| json!(reason));
    }
    let evidence_message_ids = string_array(&item["evidence_message_ids"]).collect::<Vec<_>>();
    let message_ids = if evidence_message_ids.is_empty() {
        evidence_source_messages(ctx, state)
            .iter()
            .map(|message| message.message_id.clone())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>()
    } else {
        evidence_message_ids
    };
    if !message_ids.is_empty() {
        object
            .entry("message_ids")
            .or_insert_with(|| json!(message_ids));
    }
    if let Some(quote) = trimmed_text(item["evidence_quote"].as_str(), 512) {
        object.entry("quote").or_insert_with(|| json!(quote));
    }
    evidence
}

fn memory_operation_updates_evidence(item: &Value, reason: Option<&str>) -> bool {
    reason.is_some()
        || item_has_key(item, "evidence")
        || item_has_key(item, "evidence_json")
        || item_has_key(item, "source_span")
        || item_has_key(item, "source_spans")
        || item_has_key(item, "evidence_message_ids")
        || item_has_key(item, "evidence_quote")
}

fn item_has_key(item: &Value, key: &str) -> bool {
    item.get(key).is_some_and(|value| !value.is_null())
}

fn merge_string_lists(mut existing: Vec<String>, incoming: Vec<String>) -> Vec<String> {
    for item in incoming {
        if !existing.iter().any(|current| current == &item) {
            existing.push(item);
        }
    }
    existing
}

fn memory_update_has_fields(update: &MemoryUpdate) -> bool {
    update.content.is_some()
        || update.memory_type.is_some()
        || update.truth_status.is_some()
        || update.importance.is_some()
        || update.confidence.is_some()
        || update.sensitivity.is_some()
        || update.sensitivity_category.is_some()
        || update.emotional_valence.is_some()
        || update.tags.is_some()
        || update.subjects.is_some()
        || update.evidence_message_ids.is_some()
        || update.evidence_quote.is_some()
        || update.evidence.is_some()
        || update.expires_at.is_some()
        || update.stability.is_some()
        || update.supersedes.is_some()
        || update.superseded_by.is_some()
        || update.contradiction_group.is_some()
        || update.privacy_category.is_some()
        || update.visibility_scope.is_some()
        || update.last_confirmed_at.is_some()
        || update.next_review_at.is_some()
        || update.dedupe_key.is_some()
        || update.embedding_model.is_some()
        || update.embedding_version.is_some()
        || update.embedding.is_some()
}

fn evidence_source_messages(ctx: &SessionContext, state: &SessionState) -> Vec<InboundMessage> {
    ctx.messages
        .iter()
        .chain(state.presented_injected_messages.iter())
        .chain(state.presented_read_messages.iter())
        .cloned()
        .collect()
}

fn missing_evidence_message_ids(
    ctx: &SessionContext,
    state: &SessionState,
    evidence_message_ids: &[String],
) -> Option<Vec<String>> {
    if evidence_message_ids.is_empty() {
        return None;
    }
    let messages = evidence_source_messages(ctx, state);
    let missing = evidence_message_ids
        .iter()
        .filter(|id| {
            !messages
                .iter()
                .any(|message| message.message_id.as_str() == id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    (!missing.is_empty()).then_some(missing)
}

fn source_message_for_evidence(
    ctx: &SessionContext,
    state: &SessionState,
    evidence_message_ids: &[String],
) -> Option<InboundMessage> {
    let messages = evidence_source_messages(ctx, state);
    evidence_message_ids
        .iter()
        .find_map(|id| {
            messages
                .iter()
                .find(|message| message.message_id == *id)
                .cloned()
        })
        .or_else(|| messages.into_iter().next())
}

fn stable_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

fn merge_summary_update(existing: Option<&str>, proposed: &str) -> Option<String> {
    let proposed = proposed.trim();
    if proposed.is_empty() {
        return None;
    }

    let Some(existing) = existing
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
    else {
        return Some(proposed.to_string());
    };

    let existing_normalized = normalize_summary_text(existing);
    let proposed_normalized = normalize_summary_text(proposed);
    if proposed_normalized.is_empty()
        || proposed_normalized == existing_normalized
        || existing_normalized.contains(&proposed_normalized)
    {
        return None;
    }

    if proposed.len() >= existing.len().saturating_mul(2) / 3 {
        return Some(proposed.to_string());
    }

    let novel_fragments = split_summary_fragments(proposed)
        .into_iter()
        .filter(|fragment| meaningful_summary_fragment(fragment))
        .filter(|fragment| !existing_normalized.contains(&normalize_summary_text(fragment)))
        .collect::<Vec<_>>();
    if novel_fragments.is_empty() {
        return None;
    }

    let mut merged = existing.to_string();
    for fragment in novel_fragments {
        if !ends_with_sentence_boundary(&merged) {
            merged.push('.');
        }
        merged.push(' ');
        merged.push_str(fragment);
    }
    Some(merged)
}

fn split_summary_fragments(text: &str) -> Vec<&str> {
    let mut fragments = Vec::new();
    let mut start = 0;
    for (idx, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?' | '\n') {
            let end = idx + ch.len_utf8();
            let fragment = text[start..end].trim();
            if !fragment.is_empty() {
                fragments.push(fragment);
            }
            start = end;
        }
    }
    let fragment = text[start..].trim();
    if !fragment.is_empty() {
        fragments.push(fragment);
    }
    fragments
}

fn meaningful_summary_fragment(fragment: &str) -> bool {
    let words = fragment
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count();
    words >= 2 && fragment.chars().filter(|ch| !ch.is_whitespace()).count() >= 10
}

fn normalize_summary_text(text: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                normalized.push(lower);
            }
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn ends_with_sentence_boundary(text: &str) -> bool {
    text.trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '.' | '!' | '?'))
}

fn merge_ordered_ids(mut existing: Vec<String>, proposed: Vec<String>) -> Vec<String> {
    let mut seen = existing.iter().cloned().collect::<HashSet<_>>();
    for id in proposed {
        if seen.insert(id.clone()) {
            existing.push(id);
        }
    }
    existing
}

fn array_items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
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

fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn clamp_valence(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

fn clamp_delta(value: f32, max_abs: f32) -> f32 {
    value.clamp(-max_abs, max_abs)
}

fn trimmed_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::review::tools;
    use crate::core::tools::{SessionKind, empty_delta};
    use crate::identity::{Person, PersonProfileStatus, Profile};
    use crate::state::{ActorState, Authority, GrowthConfig};
    use crate::store::{
        ActionRunRecord, MessageRole, RecallQuery, SqliteStore, Store, StoredMessage,
    };
    use async_trait::async_trait;
    use gateway::GatewayRouter;
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{ConversationId, GroupId, IdentityId, InboundMessage, ProfileId};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;
    struct EmbeddingBridge;

    #[async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: inference::AssistantMessage {
                    text: Some(String::new()),
                    reasoning_content: None,
                    tool_calls: vec![],
                },
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("noop bridge is not used by apply_review tests")
        }

        async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            anyhow::bail!("embedding endpoint unavailable")
        }
    }

    #[async_trait]
    impl OpenAiCompatibleBridge for EmbeddingBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("embedding bridge is not used for chat")
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("embedding bridge is not used for streaming")
        }

        async fn embed(&self, model: &str, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            assert_eq!(model, "embed-review");
            assert_eq!(input.len(), 1);
            Ok(vec![vec![0.1, 0.2, 0.3, 0.4]])
        }
    }

    fn router_with_failing_embedding_endpoint() -> inference::InferenceRouter {
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "chat-noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "embed-unavailable".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Embedding],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap()
    }

    fn router_with_successful_embedding_endpoint() -> inference::InferenceRouter {
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(EmbeddingBridge)),
                model: "embed-review".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Embedding],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap()
    }

    fn inbound(
        profile: &ProfileId,
        person: &PersonId,
        conversation: &ConversationId,
    ) -> InboundMessage {
        InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: Some("Sam".into()),
            reply_external_id: "local".into(),
            conversation: conversation.clone(),
            group: None,
            identity: None,
            profile: Some(profile.clone()),
            person: Some(person.clone()),
            content: "make future summaries concise".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: Value::Null,
        }
    }

    fn test_context(
        store: Arc<SqliteStore>,
        profile: &ProfileId,
        person: &PersonId,
        conversation: &ConversationId,
    ) -> (SessionContext, SessionState) {
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let router = InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap();
        let message = inbound(profile, person, conversation);
        let state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: empty_delta(Some(person.clone())),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        (
            SessionContext {
                action_id: ActionId("review-action".into()),
                kind: SessionKind::Action(ActionKind::Review),
                messages: vec![message],
                conversation: Some(conversation.clone()),
                authority: Authority::Default,
                style_directive: None,
                cancelled_note: Some("Post-turn review for action source-action".into()),
                concurrent_summaries: vec![],
                state: StateHandle::new(shared, delta_tx),
                store,
                media_store: None,
                router: Arc::new(router),
                endpoints: vec![],
                reasoning: Reasoning::Basic,
                inject_rx,
                progress: Arc::new(RwLock::new(RunningState::new())),
                max_turns: 1,
                max_action_attempts: 1,
                escalate_after: 1,
                gateway: Arc::new(GatewayRouter::new()),
                typing: Arc::new(RwLock::new(Default::default())),
                metrics: Arc::new(crate::core::ActorMetrics::default()),
                session_start: std::time::Instant::now(),
            },
            state,
        )
    }

    #[test]
    fn summary_merge_rejects_trivial_and_appends_novel_fragments() {
        let existing = "Sam likes concise summaries and deployment updates.";

        assert_eq!(merge_summary_update(Some(existing), "short"), None);
        assert_eq!(
            merge_summary_update(Some(existing), "Sam likes concise summaries."),
            None
        );
        assert_eq!(
            merge_summary_update(Some(existing), "Keeps launch checklist.").as_deref(),
            Some("Sam likes concise summaries and deployment updates. Keeps launch checklist.")
        );
        assert_eq!(
            merge_summary_update(
                Some("Sam likes concise summaries and careful deployment updates"),
                "Keeps launch checklist."
            )
            .as_deref(),
            Some(
                "Sam likes concise summaries and careful deployment updates. Keeps launch checklist."
            )
        );
        assert_eq!(
            merge_ordered_ids(
                vec!["msg-1".to_string(), "msg-2".to_string()],
                vec!["msg-2".to_string(), "msg-3".to_string()]
            ),
            vec![
                "msg-1".to_string(),
                "msg-2".to_string(),
                "msg-3".to_string()
            ]
        );
    }

    #[test]
    fn person_level_review_updates_require_verified_or_strong_likely_link() {
        let person = PersonId("person-sam".into());
        let profile = ProfileId("profile-sam".into());
        let link = |status, confidence| PersonProfileLink {
            person_id: person.clone(),
            profile_id: profile.clone(),
            status,
            confidence,
            evidence: None,
            created_at: 1000,
            updated_at: 1000,
            detached_at: None,
        };

        assert!(person_link_allows_person_level_update(&link(
            PersonProfileStatus::Verified,
            0.1,
        )));
        assert!(person_link_allows_person_level_update(&link(
            PersonProfileStatus::Likely,
            STRONG_LIKELY_PERSON_LINK_CONFIDENCE,
        )));
        assert!(!person_link_allows_person_level_update(&link(
            PersonProfileStatus::Likely,
            STRONG_LIKELY_PERSON_LINK_CONFIDENCE - 0.01,
        )));
        assert!(!person_link_allows_person_level_update(&link(
            PersonProfileStatus::Suspected,
            1.0,
        )));
    }

    #[tokio::test]
    async fn apply_review_skips_person_updates_for_weak_likely_profile_link() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-weak".into());
        let person = PersonId("person-weak".into());
        let conversation = ConversationId("relay:weak".into());
        let now = util::now();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Weak".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person.clone(),
                name: Some("Weak".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(
                &profile,
                &person,
                PersonProfileStatus::Likely,
                STRONG_LIKELY_PERSON_LINK_CONFIDENCE - 0.1,
                None,
            )
            .await
            .unwrap();
        let (ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "person_updates": [{
                    "person_id": person.0,
                    "summary": "Weak profile link should not promote to person.",
                    "comm_style": "Brief."
                }],
                "memories": [{
                    "content": "Weak profile link should not write person-scoped memory.",
                    "subjects": [{
                        "type": "person",
                        "id": person.0,
                        "role": "about",
                        "confidence": 1.0
                    }],
                    "dedupe_key": "review:test:weak-person-memory"
                }],
                "relationship_delta": [{
                    "person_id": person.0,
                    "familiarity_delta": 0.5,
                    "trust_delta": 0.5,
                    "valence_delta": 0.5,
                    "proactive_consent": "allowed",
                    "reason": "weak link should not strengthen relationship"
                }],
                "social_relations": [{
                    "person_a": person.0,
                    "person_b": "person-alice",
                    "relation": "coworker",
                    "confidence": 0.8,
                    "status": "stated",
                    "source_kind": "stated"
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["person_updates"], 0);
        assert_eq!(parsed["memories"], 0);
        assert_eq!(parsed["relationship_deltas"], 0);
        assert_eq!(parsed["social_relations"], 0);
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("strongly likely"))
        }));
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("weak person subject"))
        }));
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("relationship_delta"))
        }));
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("verified/strong person anchor"))
        }));
        let stored = store.get_person(&person).await.unwrap().unwrap();
        assert!(stored.summary.is_none());
        assert!(stored.comm_style.is_none());
        let memories = store
            .recall(&crate::store::RecallQuery::by_text(
                "Weak profile link should not write person-scoped memory.",
                5,
            ))
            .await
            .unwrap();
        assert!(memories.is_empty());
    }

    #[tokio::test]
    async fn apply_review_allows_current_person_restrictive_relationship_delta() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-weak".into());
        let person = PersonId("person-weak".into());
        let conversation = ConversationId("relay:weak".into());
        let (ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "relationship_delta": [{
                    "person_id": person.0,
                    "trust_delta": -0.5,
                    "familiarity_delta": 0.0,
                    "valence_delta": -0.5,
                    "proactive_consent": "denied",
                    "reason": "current person asked not to receive proactive outreach"
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["relationship_deltas"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());
        assert_eq!(session_state.delta.relationship_changes.len(), 1);
        assert_eq!(
            session_state.delta.relationship_changes[0].proactive_consent,
            Some(ProactiveConsent::Denied)
        );
        assert_eq!(
            session_state.delta.relationship_changes[0].trust_delta,
            -0.05
        );
    }

    #[tokio::test]
    async fn apply_review_can_create_current_group_directive() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-group".into());
        let person = PersonId("person-group".into());
        let conversation = ConversationId("relay:group".into());
        let group = GroupId("group-review".into());
        let (mut ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        ctx.messages[0].group = Some(group.clone());

        let result = apply(
            &json!({
                "directives": [{
                    "scope": "group",
                    "group_id": group.0.clone(),
                    "directive": "Use the group norm: keep release updates brief and action-oriented.",
                    "priority": 12
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["directives"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());
        let directives = store
            .get_directives_for_context(&person, &Authority::Default, Some(&group))
            .await
            .unwrap();
        assert_eq!(directives.len(), 1);
        assert_eq!(
            directives[0].scope.scope_value().as_deref(),
            Some("group-review")
        );
        assert_eq!(directives[0].set_by, person);
        assert_eq!(directives[0].priority, 12);
        assert!(directives[0].directive.contains("release updates brief"));
    }

    #[tokio::test]
    async fn non_chosen_person_review_cannot_create_directives_outside_current_scope() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-group".into());
        let person = PersonId("person-group".into());
        let conversation = ConversationId("relay:group".into());
        let group = GroupId("group-current".into());
        let (mut ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        ctx.messages[0].group = Some(group);

        let result = apply(
            &json!({
                "directives": [{
                    "scope": "group",
                    "group_id": "group-other",
                    "directive": "Use another group's norm."
                }, {
                    "scope": "person",
                    "person_id": "person-other",
                    "directive": "Use another person's norm."
                }, {
                    "scope": "global",
                    "directive": "Use a global norm."
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["directives"], 0);
        assert_eq!(parsed["skipped"].as_array().unwrap().len(), 3);
        assert!(store.list_directives().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn apply_review_requires_verified_anchor_for_relationship_preferences() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-weak".into());
        let person = PersonId("person-weak".into());
        let conversation = ConversationId("relay:weak".into());
        let (ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "relationship_delta": [{
                    "person_id": person.0,
                    "response_cadence": "reply within one business day",
                    "channel_preference": "Discord for quick coordination",
                    "reason": "current message implies durable delivery preferences"
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["relationship_deltas"], 0);
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("relationship_delta"))
        }));
        assert!(session_state.delta.relationship_changes.is_empty());
    }

    #[tokio::test]
    async fn apply_review_sets_social_trust_ceiling_for_positive_relationship_delta() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let chosen_person = PersonId("person-chosen_person".into());
        let conversation = ConversationId("relay:local".into());
        let now = util::now();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person.clone(),
                name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
        let (ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        {
            let mut actor = ctx.state.shared.actor.write().unwrap();
            actor.set_relationship_config(
                &chosen_person,
                Some(crate::state::Authority::ChosenPerson),
            );
        }

        let result = apply(
            &json!({
                "relationship_delta": [{
                    "person_id": person.0,
                    "trust_delta": 0.5,
                    "familiarity_delta": 0.2,
                    "valence_delta": 0.1,
                    "reason": "friendly but not chosen-person-connected"
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["relationship_deltas"], 1);
        assert_eq!(session_state.delta.relationship_changes.len(), 1);
        assert_eq!(
            session_state.delta.relationship_changes[0].trust_ceiling,
            Some(crate::state::Relationship::default().trust)
        );
    }

    #[tokio::test]
    async fn apply_review_skips_profile_and_person_updates_without_evidence_target() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-current".into());
        let person = PersonId("person-current".into());
        let other_profile = ProfileId("profile-other".into());
        let other_person = PersonId("person-other".into());
        let conversation = ConversationId("relay:current".into());
        store
            .add_profile(&Profile {
                id: other_profile.clone(),
                display_name: Some("Other".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: other_person.clone(),
                name: Some("Other".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(
                &other_profile,
                &other_person,
                PersonProfileStatus::Verified,
                1.0,
                None,
            )
            .await
            .unwrap();
        let (ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "profile_updates": [{
                    "profile_id": other_profile.0.clone(),
                    "summary": "Unrelated profile summary.",
                    "evidence_message_ids": ["msg-1"]
                }],
                "person_updates": [{
                    "person_id": other_person.0.clone(),
                    "summary": "Unrelated person summary.",
                    "evidence_message_ids": ["msg-1"]
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["profile_updates"], 0);
        assert_eq!(parsed["person_updates"], 0);
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("profile profile-other is not present"))
        }));
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("person person-other is not present"))
        }));
        assert!(
            store
                .get_profile(&other_profile)
                .await
                .unwrap()
                .unwrap()
                .summary
                .is_none()
        );
        assert!(
            store
                .get_person(&other_person)
                .await
                .unwrap()
                .unwrap()
                .summary
                .is_none()
        );
    }

    #[tokio::test]
    async fn apply_review_uses_cited_evidence_message_as_memory_source() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let other_profile = ProfileId("profile-alice".into());
        let other_person = PersonId("person-alice".into());
        let conversation = ConversationId("relay:local".into());
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person.clone(),
                name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
        let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
        ctx.router = Arc::new(router_with_successful_embedding_endpoint());
        let mut second = inbound(&other_profile, &other_person, &conversation);
        second.message_id = "msg-2".into();
        second.sender_external_id = "alice".into();
        second.reply_external_id = "alice".into();
        second.content = "Alice prefers release notes with chosen_people.".into();
        ctx.messages.push(second);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "create",
                    "kind": "semantic",
                    "memory_type": "preference",
                    "truth_status": "stated",
                    "content": "Alice prefers release notes with chosen_people.",
                    "evidence_message_ids": ["msg-2"],
                    "source_spans": [{
                        "message_id": "msg-2",
                        "start_char": 0,
                        "end_char": 47,
                        "quote": "Alice prefers release notes with chosen_people."
                    }]
                }],
                "social_relations": [{
                    "person_a": "person-sam",
                    "person_b": "person-alice",
                    "relation": "coworker",
                    "direction": "bidirectional",
                    "confidence": 0.8,
                    "status": "stated",
                    "source_kind": "stated",
                    "evidence_message_ids": ["msg-2"],
                    "evidence_quote": "Alice prefers release notes with chosen_people.",
                    "evidence": {"reason": "Alice stated the preference"}
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["memories"], 1);
        assert_eq!(parsed["social_relations"], 1);

        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, "profile-alice");
        assert_eq!(memory.evidence["source_spans"][0]["message_id"], "msg-2");
        assert_eq!(
            memory.evidence["source_spans"][0]["quote"],
            "Alice prefers release notes with chosen_people."
        );
        assert_eq!(memory.embedding_model.as_deref(), Some("embed-review"));
        assert_eq!(memory.embedding.as_deref(), Some(&[0.1, 0.2, 0.3, 0.4][..]));
        match memory.source {
            MemorySource::Conversation {
                profile_id,
                person_id,
                message_id,
                ..
            } => {
                assert_eq!(profile_id, Some(other_profile));
                assert_eq!(person_id, Some(other_person.clone()));
                assert_eq!(message_id.as_deref(), Some("msg-2"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }

        let relations = store
            .get_relations(&PersonId("person-sam".into()))
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].direction.as_str(), "bidirectional");
        let evidence = relations[0].evidence.as_ref().unwrap();
        assert_eq!(evidence["message_ids"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["message_ids"][0], "msg-2");
        assert_eq!(relations[0].asserted_by.as_ref(), Some(&other_person));
        assert_eq!(
            evidence["quote"],
            "Alice prefers release notes with chosen_people."
        );
        assert_eq!(
            evidence["evidence"]["reason"],
            "Alice stated the preference"
        );
    }

    #[tokio::test]
    async fn apply_review_skips_memory_with_unavailable_evidence_message_id() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "create",
                    "kind": "semantic",
                    "memory_type": "preference",
                    "truth_status": "stated",
                    "content": "Sam prefers concise release notes.",
                    "evidence_message_ids": ["msg-missing"],
                    "dedupe_key": "review:test:missing-evidence-memory"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["memories"], 0);
        assert!(state.memories_formed.is_empty());
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("unavailable evidence message ids"))
        }));
        let memories = store
            .recall(&crate::store::RecallQuery::by_text(
                "Sam prefers concise release notes.",
                5,
            ))
            .await
            .unwrap();
        assert!(memories.is_empty());
    }

    #[tokio::test]
    async fn apply_review_skips_social_relation_with_unavailable_evidence_message_id() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let now = util::now();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person.clone(),
                name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "social_relations": [{
                    "person_a": person.0,
                    "person_b": "person-alice",
                    "relation": "coworker",
                    "confidence": 0.8,
                    "status": "stated",
                    "source_kind": "stated",
                    "evidence_message_ids": ["msg-missing"],
                    "evidence_quote": "Sam said Alice is my coworker"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["social_relations"], 0);
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("unavailable evidence message ids"))
        }));
        let relations = store.get_relations(&person).await.unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn apply_review_defaults_uncertain_and_emotional_memories_to_transient() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [
                    {
                        "operation": "create",
                        "kind": "semantic",
                        "memory_type": "hypothesis",
                        "stability": "stable",
                        "content": "Sam might be joking about moving to Mars.",
                        "evidence_message_ids": ["msg-1"],
                        "dedupe_key": "review:test:hypothesis-mars"
                    },
                    {
                        "operation": "create",
                        "kind": "episodic",
                        "memory_type": "emotional_state",
                        "truth_status": "stated",
                        "stability": "stable",
                        "content": "Sam feels annoyed about launch today.",
                        "evidence_message_ids": ["msg-1"],
                        "dedupe_key": "review:test:emotion-launch"
                    }
                ]
            }),
            &ctx,
            &mut state,
        )
        .await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["memories"], 2);
        assert_eq!(state.memories_formed.len(), 2);

        let hypothesis = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hypothesis.memory_type, MemoryType::Hypothesis);
        assert_eq!(hypothesis.truth_status, TruthStatus::Inferred);
        assert_eq!(hypothesis.stability, MemoryStability::Transient);

        let emotion = store
            .get_memory(&state.memories_formed[1])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(emotion.memory_type, MemoryType::EmotionalState);
        assert_eq!(emotion.truth_status, TruthStatus::Stated);
        assert_eq!(emotion.stability, MemoryStability::Transient);
    }

    #[tokio::test]
    async fn apply_review_skips_memory_subjects_outside_evidence_profile_or_identity() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
        ctx.messages[0].identity = Some(IdentityId("identity-sam".into()));

        let result = apply(
            &json!({
                "memories": [
                    {
                        "operation": "create",
                        "content": "Sam prefers concise launch notes.",
                        "subjects": [{
                            "type": "profile",
                            "id": "profile-sam",
                            "role": "about",
                            "confidence": 1.0
                        }],
                        "evidence_message_ids": ["msg-1"],
                        "dedupe_key": "review:test:allowed-profile-subject"
                    },
                    {
                        "operation": "create",
                        "content": "Alice prefers private escalation.",
                        "subjects": [{
                            "type": "profile",
                            "id": "profile-alice",
                            "role": "about",
                            "confidence": 1.0
                        }],
                        "evidence_message_ids": ["msg-1"],
                        "dedupe_key": "review:test:blocked-profile-subject"
                    },
                    {
                        "operation": "create",
                        "content": "Sam's relay identity is the current speaker.",
                        "subjects": [{
                            "type": "identity",
                            "id": "identity-sam",
                            "role": "about",
                            "confidence": 1.0
                        }],
                        "evidence_message_ids": ["msg-1"],
                        "dedupe_key": "review:test:allowed-identity-subject"
                    },
                    {
                        "operation": "create",
                        "content": "Alice's identity prefers SMS.",
                        "subjects": [{
                            "type": "identity",
                            "id": "identity-alice",
                            "role": "about",
                            "confidence": 1.0
                        }],
                        "evidence_message_ids": ["msg-1"],
                        "dedupe_key": "review:test:blocked-identity-subject"
                    }
                ]
            }),
            &ctx,
            &mut state,
        )
        .await;

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["memories"], 2);
        assert_eq!(state.memories_formed.len(), 2);
        let skipped = parsed["skipped"].as_array().unwrap();
        assert_eq!(
            skipped
                .iter()
                .filter(|item| item
                    .as_str()
                    .is_some_and(|message| message.contains("outside review evidence")))
                .count(),
            2
        );

        let mut subject_ids = Vec::new();
        for memory_id in &state.memories_formed {
            let memory = store.get_memory(memory_id).await.unwrap().unwrap();
            subject_ids.extend(
                memory
                    .subjects
                    .iter()
                    .map(|subject| (subject.subject_type.clone(), subject.subject_id.clone())),
            );
        }
        assert!(subject_ids.contains(&(MemorySubjectType::Profile, "profile-sam".into())));
        assert!(subject_ids.contains(&(MemorySubjectType::Identity, "identity-sam".into())));
        assert!(!subject_ids.contains(&(MemorySubjectType::Profile, "profile-alice".into())));
        assert!(!subject_ids.contains(&(MemorySubjectType::Identity, "identity-alice".into())));
    }

    #[tokio::test]
    async fn apply_review_can_forget_noise_memory_with_audit_reason() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        store
            .store_memory(&Memory {
                id: MemoryId("memory-noisy-duplicate".into()),
                kind: MemoryKind::Semantic,
                content: "Noisy duplicate summary fragment.".into(),
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "forget",
                    "memory_id": "memory-noisy-duplicate",
                    "reason": "review classified this as a noisy duplicate"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        assert!(
            store
                .get_memory(&MemoryId("memory-noisy-duplicate".into()))
                .await
                .unwrap()
                .is_none()
        );
        let mutations = store
            .memory_mutations_for_memory(&MemoryId("memory-noisy-duplicate".into()), 10)
            .await
            .unwrap();
        assert_eq!(mutations[0].operation, "forget");
        assert_eq!(
            mutations[0].reason.as_deref(),
            Some("review classified this as a noisy duplicate")
        );
    }

    #[tokio::test]
    async fn apply_review_can_reinforce_existing_memory_with_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        store
            .store_memory(&Memory {
                id: MemoryId("memory-existing-preference".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Preference,
                truth_status: TruthStatus::Stated,
                content: "Sam prefers concise summaries.".into(),
                confidence: 0.4,
                importance: 0.3,
                evidence_message_ids: vec!["msg-old".into()],
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "reinforce",
                    "memory_id": "memory-existing-preference",
                    "confidence": 0.7,
                    "importance": 0.6,
                    "reason": "same preference appeared again",
                    "evidence_message_ids": ["msg-1"],
                    "evidence_quote": "make future summaries concise"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let memory = store
            .get_memory(&MemoryId("memory-existing-preference".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.confidence, 0.7);
        assert_eq!(memory.importance, 0.6);
        assert_eq!(
            memory.evidence_message_ids,
            vec!["msg-old".to_string(), "msg-1".to_string()]
        );
        assert!(memory.last_confirmed_at.is_some());
        assert_eq!(memory.evidence["operation"], "reinforce");
        assert_eq!(memory.evidence["reason"], "same preference appeared again");

        let mutations = store
            .memory_mutations_for_memory(&MemoryId("memory-existing-preference".into()), 10)
            .await
            .unwrap();
        assert_eq!(mutations[0].operation, "update");
        let fields = mutations[0].data["fields"].as_array().unwrap();
        assert!(fields.contains(&json!("confidence")));
        assert!(fields.contains(&json!("last_confirmed_at")));
        assert!(fields.contains(&json!("evidence")));
    }

    #[tokio::test]
    async fn apply_review_can_update_existing_memory_by_id() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        store
            .store_memory(&Memory {
                id: MemoryId("memory-update-target".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Fact,
                truth_status: TruthStatus::Inferred,
                content: "Sam may prefer verbose summaries.".into(),
                confidence: 0.3,
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "update",
                    "memory_id": "memory-update-target",
                    "content": "Sam prefers concise summaries.",
                    "memory_type": "preference",
                    "truth_status": "stated",
                    "confidence": 0.85,
                    "evidence_message_ids": ["msg-1"],
                    "reason": "current message corrected the prior inference"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let memory = store
            .get_memory(&MemoryId("memory-update-target".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.content, "Sam prefers concise summaries.");
        assert_eq!(memory.memory_type, MemoryType::Preference);
        assert_eq!(memory.truth_status, TruthStatus::Stated);
        assert_eq!(memory.confidence, 0.85);
        assert_eq!(memory.evidence["operation"], "update");

        let mutations = store
            .memory_mutations_for_memory(&MemoryId("memory-update-target".into()), 10)
            .await
            .unwrap();
        let fields = mutations[0].data["fields"].as_array().unwrap();
        assert!(fields.contains(&json!("content")));
        assert!(fields.contains(&json!("memory_type")));
        assert!(fields.contains(&json!("truth_status")));
    }

    #[tokio::test]
    async fn apply_review_can_mark_existing_memory_contradicted() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        store
            .store_memory(&Memory {
                id: MemoryId("memory-contradicted-target".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Fact,
                truth_status: TruthStatus::Stated,
                content: "Sam lives in Toronto.".into(),
                confidence: 0.8,
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "contradict",
                    "memory_id": "memory-contradicted-target",
                    "reason": "Sam corrected the location",
                    "evidence_message_ids": ["msg-1"],
                    "evidence_quote": "I live in Edmonton now"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let memory = store
            .get_memory(&MemoryId("memory-contradicted-target".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.truth_status, TruthStatus::Denied);
        assert!(memory.contradiction_group.is_some());
        assert_eq!(memory.evidence_message_ids, vec!["msg-1".to_string()]);
        assert_eq!(memory.evidence["operation"], "contradict");
        assert_eq!(memory.evidence["reason"], "Sam corrected the location");

        let mutations = store
            .memory_mutations_for_memory(&MemoryId("memory-contradicted-target".into()), 10)
            .await
            .unwrap();
        let fields = mutations[0].data["fields"].as_array().unwrap();
        assert!(fields.contains(&json!("truth_status")));
        assert!(fields.contains(&json!("contradiction_group")));
        assert!(fields.contains(&json!("evidence")));
    }

    #[tokio::test]
    async fn apply_review_can_supersede_existing_memory_with_replacement() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "apply_review")
            .expect("apply_review tool exists");
        assert!(
            tool.parameters["properties"]["memories"]["items"]["properties"]["operation"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("supersede"))
        );

        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        store
            .store_memory(&Memory {
                id: MemoryId("memory-old-location".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Fact,
                truth_status: TruthStatus::Stated,
                content: "Sam lives in Toronto.".into(),
                confidence: 0.8,
                evidence_message_ids: vec!["msg-old".into()],
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "supersede",
                    "memory_id": "memory-old-location",
                    "content": "Sam lives in Edmonton now.",
                    "kind": "semantic",
                    "memory_type": "fact",
                    "truth_status": "confirmed",
                    "confidence": 0.9,
                    "importance": 0.7,
                    "reason": "Sam corrected the old location",
                    "evidence_message_ids": ["msg-1"],
                    "evidence_quote": "I live in Edmonton now"
                }]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let replacement_id = state.memories_formed.last().unwrap().clone();
        assert_ne!(replacement_id, MemoryId("memory-old-location".into()));
        let replacement = store.get_memory(&replacement_id).await.unwrap().unwrap();
        assert_eq!(replacement.content, "Sam lives in Edmonton now.");
        assert_eq!(replacement.truth_status, TruthStatus::Confirmed);
        assert_eq!(
            replacement.supersedes.as_ref().map(|id| id.0.as_str()),
            Some("memory-old-location")
        );
        assert_eq!(replacement.subjects.len(), 1);
        assert_eq!(
            replacement.subjects[0].subject_type,
            MemorySubjectType::Profile
        );
        assert_eq!(replacement.subjects[0].subject_id, "profile-sam");
        assert_eq!(replacement.evidence["operation"], "supersede");
        assert_eq!(
            replacement.evidence["reason"],
            "Sam corrected the old location"
        );

        let old_memory = store
            .get_memory(&MemoryId("memory-old-location".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old_memory.truth_status, TruthStatus::Outdated);
        assert_eq!(old_memory.superseded_by, Some(replacement_id.clone()));
        assert_eq!(old_memory.evidence["operation"], "superseded");
        assert_eq!(
            old_memory.evidence["reason"],
            "Sam corrected the old location"
        );

        let mutations = store
            .memory_mutations_for_memory(&MemoryId("memory-old-location".into()), 10)
            .await
            .unwrap();
        assert_eq!(mutations[0].operation, "update");
        let fields = mutations[0].data["fields"].as_array().unwrap();
        assert!(fields.contains(&json!("truth_status")));
        assert!(fields.contains(&json!("superseded_by")));
        assert!(fields.contains(&json!("evidence")));
    }

    #[tokio::test]
    async fn apply_review_default_upsert_reuses_memory_across_review_actions() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let review_args = json!({
            "memories": [{
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Sam prefers concise future summaries.",
                "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
                "importance": 0.8,
                "confidence": 0.9,
                "evidence_message_ids": ["msg-1"]
            }]
        });

        let (mut first_ctx, mut first_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        first_ctx.action_id = ActionId("review-action-1".into());
        first_ctx.cancelled_note = Some("Post-turn review for action source-action-1".into());
        let first_result = apply(&review_args, &first_ctx, &mut first_state).await;
        let first: Value = serde_json::from_str(&first_result).unwrap();
        assert_eq!(first["status"], "applied");
        assert_eq!(first["memories"], 1);
        let first_memory_id = first_state.memories_formed[0].clone();
        let first_metrics = first_ctx.metrics.snapshot();
        assert_eq!(first_metrics.memory_created, 1);
        assert_eq!(first_metrics.memory_updated, 0);

        let (mut second_ctx, mut second_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        second_ctx.action_id = ActionId("review-action-2".into());
        second_ctx.cancelled_note = Some("Post-turn review for action source-action-2".into());
        let second_result = apply(&review_args, &second_ctx, &mut second_state).await;
        let second: Value = serde_json::from_str(&second_result).unwrap();
        assert_eq!(second["status"], "applied");
        assert_eq!(second["memories"], 1);
        assert_eq!(second_state.memories_formed[0], first_memory_id);
        let second_metrics = second_ctx.metrics.snapshot();
        assert_eq!(second_metrics.memory_created, 0);
        assert_eq!(second_metrics.memory_updated, 1);

        let memories = store
            .recall(&crate::store::RecallQuery::by_text(
                "concise future summaries",
                10,
            ))
            .await
            .unwrap();
        assert_eq!(memories.len(), 1);
        let dedupe_key = memories[0].dedupe_key.as_deref().unwrap();
        assert!(dedupe_key.starts_with("review:memory:upsert:semantic:preference:stated:"));
        assert!(!dedupe_key.contains("review-action-1"));
        assert!(!dedupe_key.contains("review-action-2"));
        assert_eq!(
            store
                .review_outputs_for_action("review-action-1")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .review_outputs_for_action("review-action-2")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn apply_review_persists_memory_without_embedding_when_embedding_endpoint_fails() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
        ctx.router = Arc::new(router_with_failing_embedding_endpoint());

        let review_args = json!({
            "memories": [{
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Sam prefers concise launch briefs.",
                "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
                "importance": 0.8,
                "confidence": 0.9,
                "evidence_message_ids": ["msg-1"]
            }]
        });

        let result = apply(&review_args, &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.content, "Sam prefers concise launch briefs.");
        assert_eq!(memory.memory_type, MemoryType::Preference);
        assert!(memory.embedding.is_none());
    }

    #[tokio::test]
    async fn apply_review_derives_sensitive_memory_policy() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let review_args = json!({
            "memories": [{
                "kind": "semantic",
                "memory_type": "fact",
                "truth_status": "stated",
                "content": "Sam mentioned a private medical follow-up.",
                "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
                "importance": 0.7,
                "confidence": 0.9,
                "sensitivity": 0.2,
                "sensitivity_category": "medical",
                "evidence_message_ids": ["msg-1"]
            }]
        });

        let result = apply(&review_args, &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["memories"], 1);

        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.privacy_category, PrivacyCategory::Sensitive);
        assert_eq!(memory.visibility_scope, VisibilityScope::Profile);
        assert!(memory.next_review_at.is_some());
    }

    #[tokio::test]
    async fn apply_review_can_create_triggered_open_loop_without_fire_at() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);

        let review_args = json!({
            "open_loops": [{
                "kind": "triggered",
                "task": "Ask how the deployment went",
                "condition": "next time Sam messages",
                "conversation_id": conversation.0,
                "dedupe_key": "review:test:triggered-followup"
            }]
        });

        let result = apply(&review_args, &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["open_loops"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        assert!(
            store
                .due_intents(util::now() + 3600, 10)
                .await
                .unwrap()
                .is_empty()
        );
        let active = store
            .active_intents_for_context(Some(&person), Some(&profile), Some(&conversation), 0, 10)
            .await
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].kind, "triggered");
        assert_eq!(active[0].task, "Ask how the deployment went");
        assert_eq!(
            active[0].condition.as_deref(),
            Some("next time Sam messages")
        );
        assert!(active[0].fire_at.is_none());
    }

    #[tokio::test]
    async fn apply_review_accepts_follow_up_open_loop_alias() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
        let now = util::now();

        let review_args = json!({
            "open_loops": [
                {
                    "kind": "follow_up",
                    "task": "Check whether the deployment finished",
                    "fire_at": now + 3600,
                    "conversation_id": conversation.0,
                    "dedupe_key": "review:test:follow-up-scheduled"
                },
                {
                    "kind": "follow_up",
                    "task": "Ask about deployment blockers",
                    "condition": "next time Sam mentions deployment",
                    "conversation_id": conversation.0,
                    "dedupe_key": "review:test:follow-up-triggered"
                }
            ]
        });

        let result = apply(&review_args, &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["open_loops"], 2);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let due = store.due_intents(now + 3600, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, "scheduled");
        assert_eq!(due[0].task, "Check whether the deployment finished");
        assert_eq!(due[0].fire_at, Some(now + 3600));
        assert!(due[0].condition.is_none());

        let active = store
            .active_intents_for_context(Some(&person), Some(&profile), Some(&conversation), 0, 10)
            .await
            .unwrap();
        let triggered = active
            .iter()
            .find(|intent| intent.task == "Ask about deployment blockers")
            .unwrap();
        assert_eq!(triggered.kind, "triggered");
        assert_eq!(
            triggered.condition.as_deref(),
            Some("next time Sam mentions deployment")
        );
        assert!(triggered.fire_at.is_none());
        assert!(active.iter().all(|intent| intent.kind != "follow_up"));
    }

    #[tokio::test]
    async fn apply_review_routes_sensitive_open_loop_to_chosen_person_approval_intent() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let chosen_person = PersonId("person-chosen_person".into());
        let conversation = ConversationId("relay:local".into());
        let (ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
        ctx.state
            .shared
            .actor
            .write()
            .unwrap()
            .set_relationship_config(&chosen_person, Some(Authority::ChosenPerson));

        let review_args = json!({
            "open_loops": [{
                "task": "Ask about the private medical update",
                "fire_at": util::now() + 3600,
                "conversation_id": conversation.0,
                "sensitive": true,
                "source_memory_id": "memory-sensitive-medical-update",
                "dedupe_key": "review:test:sensitive-followup"
            }]
        });

        let result = apply(&review_args, &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["open_loops"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let due = store.due_intents(util::now() + 1, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].person.as_ref(), Some(&chosen_person));
        assert!(due[0].chosen_person_approved);
        assert_eq!(due[0].priority, 100);
        assert!(due[0].task.contains("Review sensitive proactive outreach"));
        assert!(due[0].task.contains("Ask about the private medical update"));
        assert!(due[0].task.contains("Pending intent:"));
        assert!(due[0].task.contains("update intent"));
        assert_eq!(
            due[0].source_memory.as_ref().map(|id| id.0.as_str()),
            Some("memory-sensitive-medical-update")
        );

        let pending_id = due[0]
            .task
            .split("Pending intent: ")
            .nth(1)
            .and_then(|rest| rest.split('.').next())
            .expect("pending intent id in chosen-person approval task")
            .to_string();
        assert!(due[0].task.contains(&pending_id));
        let pending = store.get_intent(&pending_id).await.unwrap().unwrap();
        assert_eq!(pending.status, "pending_approval");
        assert_eq!(pending.task, "Ask about the private medical update");
        assert_eq!(pending.person.as_ref(), Some(&person));
        assert_eq!(pending.profile.as_ref(), Some(&profile));
        assert_eq!(pending.conversation.as_ref(), Some(&conversation));
        assert!(!pending.chosen_person_approved);
        assert_eq!(
            pending.source_memory.as_ref().map(|id| id.0.as_str()),
            Some("memory-sensitive-medical-update")
        );

        let target_intents = store
            .active_intents_for_context(Some(&person), Some(&profile), Some(&conversation), 0, 10)
            .await
            .unwrap();
        assert!(
            target_intents
                .iter()
                .all(|intent| !intent.task.contains("private medical update"))
        );

        let (mut chosen_person_ctx, mut chosen_person_state) =
            test_context(store.clone(), &profile, &chosen_person, &conversation);
        chosen_person_ctx.authority = Authority::ChosenPerson;
        let update_result = match crate::core::tools::execute(
            "update_intent",
            &json!({
                "intent_id": pending_id,
                "status": "active"
            }),
            &chosen_person_ctx,
            &mut chosen_person_state,
        )
        .await
        {
            crate::core::tools::ToolOutcome::Result(result) => result,
            crate::core::tools::ToolOutcome::Decision(_) => {
                panic!("update_intent should produce a tool result")
            }
        };
        let parsed_update: Value = serde_json::from_str(&update_result).unwrap();
        assert_eq!(parsed_update["status"], "updated");
        let approved = store.get_intent(&pending.id).await.unwrap().unwrap();
        assert_eq!(approved.status, "active");
        assert!(approved.chosen_person_approved);
    }

    #[tokio::test]
    async fn chosen_person_review_can_create_third_party_open_loop() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-chosen_person".into());
        let person = PersonId("person-chosen_person".into());
        let conversation = ConversationId("relay:chosen_person".into());
        let (mut ctx, mut state) = test_context(store.clone(), &profile, &person, &conversation);
        ctx.authority = Authority::ChosenPerson;

        let now = util::now();
        let review_args = json!({
            "open_loops": [{
                "task": "Remind Alice to bring the deployment checklist",
                "fire_at": now + 3600,
                "person_id": "person-alice",
                "profile_id": "profile-alice",
                "conversation_id": "relay:alice",
                "dedupe_key": "chosen_person:remind-alice-checklist"
            }]
        });

        let result = apply(&review_args, &ctx, &mut state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["open_loops"], 1);
        assert!(parsed["skipped"].as_array().unwrap().is_empty());

        let due = store.due_intents(now + 3600, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].person.as_ref(),
            Some(&PersonId("person-alice".into()))
        );
        assert_eq!(
            due[0].profile.as_ref(),
            Some(&ProfileId("profile-alice".into()))
        );
        assert_eq!(
            due[0].conversation.as_ref(),
            Some(&ConversationId("relay:alice".into()))
        );
        assert!(due[0].chosen_person_approved);
    }

    #[tokio::test]
    async fn apply_review_writes_structured_outputs() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let person = PersonId("person-sam".into());
        let conversation = ConversationId("relay:local".into());
        let now = util::now();
        store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Sam".into()),
                summary: Some("Sam likes concise summaries and deployment updates.".into()),
                comm_style: None,
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person.clone(),
                name: Some("Sam".into()),
                summary: None,
                comm_style: None,
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(&profile, &person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
        store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "make future summaries concise".into(),
                    identity: None,
                    profile: Some(profile.clone()),
                    person: Some(person.clone()),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-1".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .start_action_run(&ActionRunRecord {
                action_id: "source-action".into(),
                kind: "respond".into(),
                task: "Respond to Sam".into(),
                conversation: Some(conversation.clone()),
                started_at: now - 20,
                ended_at: None,
                status: "running".into(),
                responded: false,
                attempts: 0,
            })
            .await
            .unwrap();
        store
            .finish_action_run(
                "source-action",
                now - 10,
                "completed",
                true,
                1,
                vec![],
                vec![],
            )
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("old-summary-preference".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Preference,
                truth_status: TruthStatus::Stated,
                content: "Sam prefers long future summaries.".into(),
                source: MemorySource::Reflection,
                importance: 0.7,
                confidence: 0.8,
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut session_state) =
            test_context(store.clone(), &profile, &person, &conversation);

        let review_args = json!({
            "profile_updates": [{
                "profile_id": profile.0,
                "summary": "short",
                "comm_style": "Concise and practical."
            }],
            "person_updates": [{
                "person_id": person.0,
                "summary": "Sam prefers concise operational updates across contexts.",
                "comm_style": "Direct, brief, practical."
            }],
            "memories": [{
                "operation": "upsert",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "content": "Sam prefers concise future summaries.",
                "subjects": [{"type": "profile", "id": "profile-sam", "role": "about", "confidence": 1.0}],
                "importance": 0.8,
                "confidence": 0.9,
                "evidence_message_ids": ["msg-1"],
                "supersedes": "old-summary-preference",
                "dedupe_key": "preference:profile-sam:concise-summaries"
            }],
            "relationship_delta": [{
                "person_id": person.0,
                "familiarity_delta": 0.5,
                "trust_delta": 0.5,
                "valence_delta": 0.5,
                "closeness_delta": 0.5,
                "reliability_delta": 0.5,
                "reciprocity_delta": 0.5,
                "conflict_delta": -0.5,
                "proactive_consent": "allowed",
                "response_cadence": "reply within one business day",
                "channel_preference": "Discord for quick deployment coordination",
                "reason": "brief friendly exchange"
            }],
            "social_relations": [{
                "person_a": "person-sam",
                "person_b": "person-alice",
                "relation": "coworker",
                "confidence": 0.8,
                "status": "stated",
                "source_kind": "stated",
                "evidence": {"quote": "Alice is my coworker"}
            }],
            "open_loops": [{
                "task": "Ask whether concise summaries helped",
                "fire_at": now + 3600,
                "conversation_id": conversation.0,
                "source_memory_id": "old-summary-preference",
                "dedupe_key": "review:test:followup"
            }, {
                "task": "Ask about the private medical update",
                "fire_at": now + 3600,
                "conversation_id": conversation.0,
                "sensitive": true,
                "dedupe_key": "review:test:sensitive-followup"
            }, {
                "task": "Ask Alice whether Sam's summary preference applies to her too",
                "fire_at": now + 3600,
                "person_id": "person-alice",
                "dedupe_key": "review:test:third-party-followup"
            }],
            "conversation_summary": {
                "conversation_id": conversation.0,
                "summary": "Sam asked for concise future summaries.",
                "covered_message_ids": ["msg-1"]
            }
        });

        let result = apply(&review_args, &ctx, &mut session_state).await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["profile_updates"], 1);
        assert_eq!(parsed["person_updates"], 1);
        assert_eq!(parsed["memories"], 1);
        assert_eq!(parsed["relationship_deltas"], 1);
        assert_eq!(parsed["social_relations"], 1);
        assert_eq!(parsed["open_loops"], 1);
        assert_eq!(parsed["conversation_summaries"], 1);
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("requires chosen-person approval"))
        }));
        assert!(parsed["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("unverified third-party"))
        }));
        let review_outputs = store
            .review_outputs_for_action("review-action")
            .await
            .unwrap();
        assert_eq!(review_outputs.len(), 1);
        assert_eq!(
            review_outputs[0].source_action_id.as_deref(),
            Some("source-action")
        );
        let metrics = ctx.metrics.snapshot();
        assert_eq!(metrics.review_outputs, 1);
        assert!(metrics.review_latency_ms_total >= 10_000);
        assert_eq!(
            review_outputs[0].input["memories"][0]["dedupe_key"],
            "preference:profile-sam:concise-summaries"
        );
        assert_eq!(
            review_outputs[0].input["memories"][0]["content"],
            "[redacted]"
        );
        assert_eq!(
            review_outputs[0].input["profile_updates"][0]["summary"],
            "[redacted]"
        );
        assert_eq!(
            review_outputs[0].input["conversation_summary"]["summary"],
            "[redacted]"
        );
        assert_eq!(
            review_outputs[0].input["relationship_delta"][0]["response_cadence"],
            "[redacted]"
        );
        assert_eq!(
            review_outputs[0].input["relationship_delta"][0]["channel_preference"],
            "[redacted]"
        );
        assert_eq!(review_outputs[0].result["memories"], 1);

        let updated_profile = store.get_profile(&profile).await.unwrap().unwrap();
        assert_eq!(
            updated_profile.summary.as_deref(),
            Some("Sam likes concise summaries and deployment updates.")
        );
        assert_eq!(
            updated_profile.comm_style.as_deref(),
            Some("Concise and practical.")
        );
        let updated_person = store.get_person(&person).await.unwrap().unwrap();
        assert_eq!(
            updated_person.summary.as_deref(),
            Some("Sam prefers concise operational updates across contexts.")
        );
        assert_eq!(
            updated_person.comm_style.as_deref(),
            Some("Direct, brief, practical.")
        );

        let memories = store
            .recall(&crate::store::RecallQuery::by_text("concise summaries", 5))
            .await
            .unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(
            memories[0].dedupe_key.as_deref(),
            Some("preference:profile-sam:concise-summaries")
        );
        assert_eq!(
            memories[0].supersedes.as_ref().map(|id| id.0.as_str()),
            Some("old-summary-preference")
        );
        let old_memory = store
            .get_memory(&MemoryId("old-summary-preference".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old_memory.truth_status, TruthStatus::Outdated);
        assert_eq!(
            old_memory.superseded_by,
            Some(session_state.memories_formed[0].clone())
        );
        assert_eq!(session_state.memories_formed.len(), 1);
        assert_eq!(
            session_state.delta.relationship_changes[0].trust_delta,
            0.05
        );
        assert_eq!(
            session_state.delta.relationship_changes[0].familiarity_delta,
            0.1
        );
        assert_eq!(
            session_state.delta.relationship_changes[0].valence_delta,
            0.1
        );
        assert_eq!(
            session_state.delta.relationship_changes[0].proactive_consent,
            Some(ProactiveConsent::Allowed)
        );
        assert_eq!(
            session_state.delta.relationship_changes[0]
                .response_cadence
                .as_deref(),
            Some("reply within one business day")
        );
        assert_eq!(
            session_state.delta.relationship_changes[0]
                .channel_preference
                .as_deref(),
            Some("Discord for quick deployment coordination")
        );
        assert_eq!(session_state.delta.relationship_signal_updates.len(), 1);
        assert_eq!(
            session_state.delta.relationship_signal_updates[0].closeness_delta,
            0.05
        );
        assert_eq!(
            session_state.delta.relationship_signal_updates[0].reliability_delta,
            0.05
        );
        assert_eq!(
            session_state.delta.relationship_signal_updates[0].reciprocity_delta,
            0.05
        );
        assert_eq!(
            session_state.delta.relationship_signal_updates[0].conflict_delta,
            -0.05
        );
        let due_intents = store.due_intents(now + 3600, 10).await.unwrap();
        assert_eq!(due_intents.len(), 1);
        assert_eq!(
            due_intents[0]
                .source_memory
                .as_ref()
                .map(|id| id.0.as_str()),
            Some("old-summary-preference")
        );
        let relations = store.get_relations(&person).await.unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].person_b, PersonId("person-alice".into()));
        assert_eq!(relations[0].relation.as_str(), "coworker");
        assert_eq!(relations[0].confidence, 0.8);
        assert_eq!(relations[0].status, RelationStatus::Stated);
        assert_eq!(relations[0].source_kind, RelationSource::Stated);
        assert_eq!(relations[0].asserted_by.as_ref(), Some(&person));
        assert_eq!(
            relations[0].evidence.as_ref().unwrap()["message_ids"][0],
            "msg-1"
        );
        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(
            conversations[0].summary.as_deref(),
            Some("Sam asked for concise future summaries.")
        );
        assert_eq!(conversations[0].summary_version, 1);

        let (retry_ctx, mut retry_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        let duplicate_result = apply(&review_args, &retry_ctx, &mut retry_state).await;
        let duplicate: Value = serde_json::from_str(&duplicate_result).unwrap();
        assert_eq!(duplicate["status"], "already_applied");
        assert_eq!(duplicate["memories"], 1);
        assert_eq!(duplicate["relationship_deltas"], 1);
        assert_eq!(duplicate["social_relations"], 1);
        assert_eq!(duplicate["open_loops"], 1);
        assert_eq!(session_state.memories_formed.len(), 1);
        assert_eq!(session_state.delta.relationship_changes.len(), 1);
        assert_eq!(retry_state.memories_formed.len(), 0);
        assert_eq!(retry_state.delta.relationship_changes.len(), 0);
        assert_eq!(store.due_intents(now + 3600, 10).await.unwrap().len(), 1);
        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(conversations[0].summary_version, 1);
        let review_outputs = store
            .review_outputs_for_action("review-action")
            .await
            .unwrap();
        assert_eq!(review_outputs.len(), 1);
        assert_eq!(review_outputs[0].result["status"], "applied");
        let review_outputs_for_source = store
            .review_outputs_for_source_action("source-action")
            .await
            .unwrap();
        assert_eq!(review_outputs_for_source.len(), 1);

        let (mut duplicate_source_ctx, mut duplicate_source_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        duplicate_source_ctx.action_id = ActionId("review-action-duplicate-source".into());
        duplicate_source_ctx.cancelled_note =
            Some("Post-turn review for action source-action".into());
        let duplicate_source_result = apply(
            &review_args,
            &duplicate_source_ctx,
            &mut duplicate_source_state,
        )
        .await;
        let duplicate_source: Value = serde_json::from_str(&duplicate_source_result).unwrap();
        assert_eq!(duplicate_source["status"], "already_applied");
        assert_eq!(duplicate_source["memories"], 1);
        assert_eq!(duplicate_source_state.memories_formed.len(), 0);
        assert_eq!(duplicate_source_state.delta.relationship_changes.len(), 0);
        assert_eq!(store.due_intents(now + 3600, 10).await.unwrap().len(), 1);
        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(conversations[0].summary_version, 1);
        assert_eq!(
            store
                .review_outputs_for_action("review-action-duplicate-source")
                .await
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            store
                .review_outputs_for_source_action("source-action")
                .await
                .unwrap()
                .len(),
            1
        );

        let (mut summary_ctx, mut summary_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        summary_ctx.action_id = ActionId("review-action-summary-merge".into());
        summary_ctx.cancelled_note =
            Some("Post-turn review for action source-action-summary-merge".into());
        let summary_args = json!({
            "conversation_summary": {
                "conversation_id": conversation.0,
                "summary": "Uses checklist.",
                "covered_message_ids": ["msg-1", "msg-2"]
            }
        });
        let summary_result = apply(&summary_args, &summary_ctx, &mut summary_state).await;
        let summary_parsed: Value = serde_json::from_str(&summary_result).unwrap();
        assert_eq!(summary_parsed["conversation_summaries"], 1);
        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(
            conversations[0].summary.as_deref(),
            Some("Sam asked for concise future summaries. Uses checklist.")
        );
        assert_eq!(
            conversations[0].summary_covered_message_ids,
            vec!["msg-1".to_string(), "msg-2".to_string()]
        );
        assert_eq!(conversations[0].summary_version, 2);

        let (mut redundant_ctx, mut redundant_state) =
            test_context(store.clone(), &profile, &person, &conversation);
        redundant_ctx.action_id = ActionId("review-action-summary-redundant".into());
        redundant_ctx.cancelled_note =
            Some("Post-turn review for action source-action-summary-redundant".into());
        let redundant_args = json!({
            "conversation_summary": {
                "conversation_id": conversation.0,
                "summary": "Sam asked for concise future summaries.",
                "covered_message_ids": ["msg-1", "msg-2"]
            }
        });
        let redundant_result = apply(&redundant_args, &redundant_ctx, &mut redundant_state).await;
        let redundant: Value = serde_json::from_str(&redundant_result).unwrap();
        assert_eq!(redundant["conversation_summaries"], 0);
        assert!(redundant["skipped"].as_array().unwrap().iter().any(|item| {
            item.as_str()
                .is_some_and(|message| message.contains("had no new fields"))
        }));
        let conversations = store.list_conversations().await.unwrap();
        assert_eq!(conversations[0].summary_version, 2);
    }

    #[tokio::test]
    async fn apply_review_uses_presented_injected_message_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let source_profile = ProfileId("profile-source".into());
        let source_person = PersonId("person-source".into());
        let injected_profile = ProfileId("profile-injected".into());
        let injected_person = PersonId("person-injected".into());
        let source_conversation = ConversationId("relay:source".into());
        let injected_conversation = ConversationId("relay:injected".into());
        let now = util::now();
        for (profile, person, name) in [
            (&source_profile, &source_person, "Source"),
            (&injected_profile, &injected_person, "Injected"),
        ] {
            store
                .add_profile(&Profile {
                    id: profile.clone(),
                    display_name: Some(name.into()),
                    summary: None,
                    comm_style: None,
                    first_seen: now,
                    last_seen: now,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
            store
                .add_person(&Person {
                    id: person.clone(),
                    name: Some(name.into()),
                    summary: None,
                    comm_style: None,
                    first_seen: now,
                    last_seen: now,
                })
                .await
                .unwrap();
            store
                .attach_profile_to_person(profile, person, PersonProfileStatus::Verified, 1.0, None)
                .await
                .unwrap();
        }
        let (mut ctx, mut session_state) = test_context(
            store.clone(),
            &source_profile,
            &source_person,
            &source_conversation,
        );
        ctx.action_id = ActionId("review-injected-evidence".into());
        ctx.conversation = None;
        let mut injected = inbound(&injected_profile, &injected_person, &injected_conversation);
        injected.message_id = "msg-injected".into();
        injected.content =
            "Injected says release notes need chosen_people and rollback paths.".into();
        injected.timestamp = 1001;
        session_state.presented_injected_messages.push(injected);

        let result = apply(
            &json!({
                "profile_updates": [{
                    "profile_id": injected_profile.0.clone(),
                    "summary": "Injected profile wants release notes with chosen_people and rollback paths.",
                    "evidence_message_ids": ["msg-injected"]
                }],
                "person_updates": [{
                    "person_id": injected_person.0.clone(),
                    "summary": "Injected person wants release notes with chosen_people and rollback paths.",
                    "evidence_message_ids": ["msg-injected"]
                }],
                "memories": [{
                    "operation": "upsert",
                    "kind": "semantic",
                    "memory_type": "preference",
                    "truth_status": "stated",
                    "content": "Injected person prefers release notes with chosen_people and rollback paths.",
                    "evidence_message_ids": ["msg-injected"],
                    "dedupe_key": "preference:profile-injected:release-note-chosen_person-rollback"
                }],
                "relationship_delta": [{
                    "person_id": injected_person.0.clone(),
                    "familiarity_delta": 0.05,
                    "reason": "injected message was presented during review"
                }],
                "social_relations": [{
                    "person_a": source_person.0.clone(),
                    "person_b": injected_person.0.clone(),
                    "relation": "coworker",
                    "confidence": 0.8,
                    "status": "stated",
                    "source_kind": "stated",
                    "evidence_message_ids": ["msg-injected"]
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["status"], "applied");
        assert_eq!(parsed["profile_updates"], 1);
        assert_eq!(parsed["person_updates"], 1);
        assert_eq!(parsed["memories"], 1);
        assert_eq!(parsed["relationship_deltas"], 1);
        assert_eq!(parsed["social_relations"], 1);
        assert_eq!(parsed["skipped"].as_array().unwrap().len(), 0);

        let updated_profile = store.get_profile(&injected_profile).await.unwrap().unwrap();
        assert_eq!(
            updated_profile.summary.as_deref(),
            Some("Injected profile wants release notes with chosen_people and rollback paths.")
        );
        let updated_person = store.get_person(&injected_person).await.unwrap().unwrap();
        assert_eq!(
            updated_person.summary.as_deref(),
            Some("Injected person wants release notes with chosen_people and rollback paths.")
        );

        let memory = store
            .get_memory(&session_state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, injected_profile.0.as_str());
        match memory.source {
            MemorySource::Conversation {
                conversation_id,
                profile_id,
                person_id,
                message_id,
                ..
            } => {
                assert_eq!(conversation_id, injected_conversation);
                assert_eq!(profile_id, Some(injected_profile.clone()));
                assert_eq!(person_id, Some(injected_person.clone()));
                assert_eq!(message_id.as_deref(), Some("msg-injected"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }
        assert_eq!(
            session_state.delta.relationship_changes[0].person,
            injected_person.clone()
        );
        let relations = store.get_relations(&source_person).await.unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].person_b, injected_person);
        assert_eq!(
            relations[0].asserted_by.as_ref(),
            Some(&PersonId("person-injected".into()))
        );
        assert_eq!(
            relations[0].evidence.as_ref().unwrap()["message_ids"][0],
            "msg-injected"
        );
    }

    #[tokio::test]
    async fn apply_review_uses_presented_read_message_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let source_profile = ProfileId("profile-source".into());
        let source_person = PersonId("person-source".into());
        let read_profile = ProfileId("profile-read".into());
        let read_person = PersonId("person-read".into());
        let source_conversation = ConversationId("relay:source".into());
        let read_conversation = ConversationId("relay:read".into());
        let now = util::now();
        for (profile, person, name) in [
            (&source_profile, &source_person, "Source"),
            (&read_profile, &read_person, "Read"),
        ] {
            store
                .add_profile(&Profile {
                    id: profile.clone(),
                    display_name: Some(name.into()),
                    summary: None,
                    comm_style: None,
                    first_seen: now,
                    last_seen: now,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
            store
                .add_person(&Person {
                    id: person.clone(),
                    name: Some(name.into()),
                    summary: None,
                    comm_style: None,
                    first_seen: now,
                    last_seen: now,
                })
                .await
                .unwrap();
            store
                .attach_profile_to_person(profile, person, PersonProfileStatus::Verified, 1.0, None)
                .await
                .unwrap();
        }
        let (mut ctx, mut session_state) = test_context(
            store.clone(),
            &source_profile,
            &source_person,
            &source_conversation,
        );
        ctx.action_id = ActionId("review-read-evidence".into());
        let mut read = inbound(&read_profile, &read_person, &read_conversation);
        read.message_id = "msg-read".into();
        read.content = "Read person prefers concise incident notes.".into();
        read.timestamp = 1001;
        session_state.presented_read_messages.push(read);

        let result = apply(
            &json!({
                "memories": [{
                    "operation": "upsert",
                    "kind": "semantic",
                    "memory_type": "preference",
                    "truth_status": "stated",
                    "content": "Read person prefers concise incident notes.",
                    "evidence_message_ids": ["msg-read"],
                    "dedupe_key": "preference:profile-read:concise-incident-notes"
                }]
            }),
            &ctx,
            &mut session_state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["memories"], 1);
        let memories = store
            .recall(&RecallQuery::by_text("concise incident notes", 5))
            .await
            .unwrap();
        assert_eq!(memories.len(), 1);
        let memory = &memories[0];
        assert_eq!(memory.evidence_message_ids, vec!["msg-read"]);
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, read_profile.0);
        match &memory.source {
            MemorySource::Conversation { message_id, .. } => {
                assert_eq!(message_id.as_deref(), Some("msg-read"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }
    }
}
