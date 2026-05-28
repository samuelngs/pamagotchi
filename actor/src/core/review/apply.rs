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
mod tests;
