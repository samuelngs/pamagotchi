use super::*;

pub(super) async fn apply_memories(
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
    if matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
        || !item_has_key(item, "subjects")
    {
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
