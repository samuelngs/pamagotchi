use super::*;

pub(super) async fn debug_snapshot(
    store: &dyn Store,
    metrics: &ActorMetrics,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let limit = limit.clamp(1, 100);
    let conversations = store.list_conversations().await?;
    let persons = store.list_persons().await?;
    let profiles = store.list_profiles().await?;
    let memories = store.debug_recent_memories(limit).await?;
    let memory_subjects = store.debug_memory_subjects(limit).await?;
    let profile_identity_links = store.debug_profile_identity_links(limit).await?;
    let person_profile_links = store.debug_person_profile_links(limit).await?;
    let groups = store.debug_groups(limit).await?;
    let intents = store.debug_active_intents(limit).await?;
    let review_outputs = store.debug_recent_review_outputs(limit).await?;
    let review_jobs = store.debug_recent_review_jobs(limit).await?;
    let raw_action_runs = store.debug_recent_action_runs(limit).await?;
    let mut action_runs = Vec::with_capacity(raw_action_runs.len());
    let mut action_traces = Vec::with_capacity(raw_action_runs.len());
    for run in &raw_action_runs {
        let run_value = serde_json::to_value(run)?;
        action_runs.push(redact_debug_trace_value(&run_value));
        let trace = serde_json::to_value(store.action_transcript(&run.action_id).await?)?;
        action_traces.push(redact_debug_trace_value(&trace));
    }
    let memory_mutations = store.debug_recent_memory_mutations(limit).await?;
    let failed_events = store.debug_recent_failed_events(limit).await?;
    let directives = store.list_directives().await?;
    let pending_claims = store.get_pending_claims().await?;

    Ok(serde_json::json!({
        "generated_at": now_secs(),
        "limit": limit,
        "metrics": metrics.snapshot(),
        "conversations": conversations,
        "persons": persons,
        "profiles": profiles,
        "profile_identity_links": profile_identity_links,
        "person_profile_links": person_profile_links,
        "groups": groups,
        "memories": memories,
        "memory_subjects": memory_subjects,
        "intents": intents,
        "review_outputs": review_outputs,
        "review_jobs": review_jobs,
        "action_runs": action_runs,
        "action_traces": action_traces,
        "memory_mutations": memory_mutations,
        "failed_events": failed_events,
        "directives": directives,
        "pending_identity_claims": pending_claims,
    }))
}

fn redact_debug_trace_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if should_redact_debug_trace_key(key) {
                        serde_json::Value::String("[redacted]".into())
                    } else {
                        redact_debug_trace_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_debug_trace_value).collect())
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                match serde_json::from_str::<serde_json::Value>(text) {
                    Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                        serde_json::Value::String(redact_debug_trace_value(&parsed).to_string())
                    }
                    _ => value.clone(),
                }
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn should_redact_debug_trace_key(key: &str) -> bool {
    matches!(
        key,
        "content"
            | "text"
            | "summary"
            | "comm_style"
            | "evidence_quote"
            | "reason"
            | "task"
            | "external_id"
            | "sender_external_id"
            | "reply_external_id"
            | "source_message_id"
            | "media_url"
            | "url"
            | "raw_arguments"
            | "error"
    )
}
