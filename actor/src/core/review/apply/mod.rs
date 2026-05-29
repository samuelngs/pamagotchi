use crate::core::tools::{SessionContext, SessionState, permission, util};
use crate::identity::{
    Relation, RelationDirection, RelationSource, RelationStatus, SocialRelation,
};
use crate::state::{
    BehaviorDirective, DirectiveScope, ProactiveConsent, RelationshipChange,
    RelationshipSignalUpdate, RelationshipStanding,
};
use crate::store::{
    IntentRecord, Memory, MemoryKind, MemorySource, MemoryStability, MemorySubject,
    MemorySubjectType, MemoryType, MemoryUpdate, PrivacyCategory, ReviewOutputAudit, TruthStatus,
    VisibilityScope, memory_privacy_policy, memory_stability_policy, memory_truth_status_policy,
    sensitive_memory_next_review_at,
};
use protocol::{ChannelId, ConversationId, InboundMessage, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};
use std::collections::HashSet;

mod conversation;
mod directives;
mod memory;
mod open_loops;
mod profile;
mod relationships;

use conversation::apply_conversation_summary;
use directives::apply_directives;
use memory::apply_memories;
use open_loops::apply_open_loops;
use profile::{apply_person_updates, apply_profile_updates};
use relationships::{apply_relationship_deltas, apply_social_relations};

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

fn profile_update_target_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    item: &Value,
    profile: &ProfileId,
) -> bool {
    matches!(
        ctx.relationship_standing,
        crate::state::RelationshipStanding::ChosenHuman
    ) || evidence_message_matches_target(item, ctx, state, |message| {
        message.profile.as_ref() == Some(profile)
    })
}

fn person_update_target_allowed(
    ctx: &SessionContext,
    state: &SessionState,
    item: &Value,
    person: &PersonId,
) -> bool {
    matches!(
        ctx.relationship_standing,
        crate::state::RelationshipStanding::ChosenHuman
    ) || evidence_message_matches_target(item, ctx, state, |message| {
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

async fn person_has_verified_or_strong_profile_context(
    ctx: &SessionContext,
    person: &PersonId,
) -> bool {
    permission::person_has_verified_or_strong_profile_context(ctx, person)
        .await
        .unwrap_or(false)
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
