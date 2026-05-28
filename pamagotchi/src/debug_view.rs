use serde_json::Value;
use std::fmt::Write;

pub fn format_snapshot(snapshot: &Value) -> String {
    let mut out = String::new();
    let generated_at = field(snapshot, "generated_at").unwrap_or_else(|| "-".into());
    let limit = field(snapshot, "limit").unwrap_or_else(|| "-".into());

    writeln!(out, "Debug snapshot").ok();
    writeln!(out, "generated_at: {generated_at}  limit: {limit}").ok();

    append_metrics(&mut out, snapshot.get("metrics"));
    append_records(&mut out, "Persons", snapshot, "persons", render_person);
    append_records(&mut out, "Profiles", snapshot, "profiles", render_profile);
    append_records(&mut out, "Groups", snapshot, "groups", render_group);
    append_records(
        &mut out,
        "Memories by subject",
        snapshot,
        "memory_subjects",
        render_memory_subject,
    );
    append_records(
        &mut out,
        "Recent memories",
        snapshot,
        "memories",
        render_memory,
    );
    append_records(&mut out, "Intents", snapshot, "intents", render_intent);
    append_records(
        &mut out,
        "Review jobs",
        snapshot,
        "review_jobs",
        render_review_job,
    );
    append_records(
        &mut out,
        "Action traces",
        snapshot,
        "action_traces",
        render_action_trace,
    );
    append_records(
        &mut out,
        "Memory mutations",
        snapshot,
        "memory_mutations",
        render_memory_mutation,
    );
    append_records(
        &mut out,
        "Failed events",
        snapshot,
        "failed_events",
        render_failed_event,
    );

    out
}

fn append_metrics(out: &mut String, metrics: Option<&Value>) {
    writeln!(out).ok();
    writeln!(out, "Metrics").ok();
    let Some(metrics) = metrics else {
        writeln!(out, "- unavailable").ok();
        return;
    };

    let events = compact_pairs(
        metrics,
        &[
            ("events_received", "received"),
            ("events_dropped", "dropped"),
            ("events_deferred", "deferred"),
            ("event_queue_depth", "queue"),
        ],
    );
    let actions = compact_pairs(
        metrics,
        &[
            ("actions_spawned", "spawned"),
            ("actions_completed", "completed"),
            ("actions_failed", "failed"),
            ("running_actions", "running"),
        ],
    );
    let memory = compact_pairs(
        metrics,
        &[
            ("memory_created", "created"),
            ("memory_updated", "updated"),
            ("memory_superseded", "superseded"),
            ("review_outputs", "reviews"),
        ],
    );
    let delivery = compact_pairs(
        metrics,
        &[
            ("outbound_delivery_success", "sent"),
            ("outbound_delivery_failure", "failed"),
            ("injection_successes", "injected"),
            ("injection_failures", "inject_failed"),
        ],
    );
    let prompt = compact_pairs(
        metrics,
        &[
            ("prompt_turns_with_usage", "turns"),
            ("prompt_input_tokens", "input_tokens"),
            ("prompt_output_tokens", "output_tokens"),
        ],
    );
    let app_server = compact_pairs(
        metrics,
        &[
            ("app_server_tool_calls", "calls"),
            ("app_server_tool_latency_ms_total", "latency_ms"),
        ],
    );
    let malformed = field(metrics, "malformed_tool_json").unwrap_or_else(|| "0".into());

    writeln!(out, "- events: {events}").ok();
    writeln!(out, "- actions: {actions}").ok();
    writeln!(out, "- memory/review: {memory}").ok();
    writeln!(out, "- delivery: {delivery}").ok();
    writeln!(out, "- prompt usage: {prompt}").ok();
    writeln!(out, "- app-server tools: {app_server}").ok();
    writeln!(out, "- malformed tool JSON: {malformed}").ok();
    if let Some(tools) = tool_metrics(metrics) {
        writeln!(out, "- tool calls: {tools}").ok();
    }
    if let Some(by_model) = metrics
        .get("malformed_tool_json_by_model")
        .and_then(Value::as_object)
        .filter(|models| !models.is_empty())
    {
        let mut models = by_model
            .iter()
            .filter_map(|(model, count)| scalar(count).map(|count| format!("{model}={count}")))
            .collect::<Vec<_>>();
        models.sort();
        writeln!(out, "- malformed by model: {}", models.join(" ")).ok();
    }
    if let Some(avg) = average(metrics, "review_latency_ms_total", "review_outputs") {
        writeln!(out, "- avg_review_latency_ms: {avg:.1}").ok();
    }
    if let Some(avg) = average(metrics, "prompt_input_tokens", "prompt_turns_with_usage") {
        writeln!(out, "- avg_prompt_input_tokens: {avg:.1}").ok();
    }
    if let Some(avg) = average(metrics, "prompt_output_tokens", "prompt_turns_with_usage") {
        writeln!(out, "- avg_prompt_output_tokens: {avg:.1}").ok();
    }
    if let Some(avg) = average(metrics, "recall_latency_ms_total", "recall_calls") {
        writeln!(out, "- avg_recall_latency_ms: {avg:.1}").ok();
    }
    if let Some(avg) = average(metrics, "recall_result_count", "recall_calls") {
        writeln!(out, "- avg_recall_results: {avg:.1}").ok();
    }
    if let Some(avg) = average(
        metrics,
        "app_server_tool_latency_ms_total",
        "app_server_tool_calls",
    ) {
        writeln!(out, "- avg_app_server_tool_latency_ms: {avg:.1}").ok();
    }
}

fn append_records<F>(out: &mut String, title: &str, snapshot: &Value, key: &str, render: F)
where
    F: Fn(&Value) -> String,
{
    writeln!(out).ok();
    match snapshot.get(key).and_then(Value::as_array) {
        Some(records) if records.is_empty() => {
            writeln!(out, "{title} (0)").ok();
            writeln!(out, "- none").ok();
        }
        Some(records) => {
            writeln!(out, "{title} ({})", records.len()).ok();
            for record in records {
                writeln!(out, "- {}", render(record)).ok();
            }
        }
        None => {
            writeln!(out, "{title}").ok();
            writeln!(out, "- unavailable").ok();
        }
    }
}

fn render_person(value: &Value) -> String {
    let id = field(value, "id").unwrap_or_else(|| "<unknown>".into());
    let name = field(value, "name").unwrap_or_else(|| "unnamed".into());
    let summary = field(value, "summary")
        .map(|summary| format!(" summary=\"{}\"", truncate(&summary, 96)))
        .unwrap_or_default();
    format!("{id} name=\"{}\"{summary}", truncate(&name, 48))
}

fn render_profile(value: &Value) -> String {
    let id = field(value, "id").unwrap_or_else(|| "<unknown>".into());
    let display = field(value, "display_name").unwrap_or_else(|| "unnamed".into());
    let summary = field(value, "summary")
        .map(|summary| format!(" summary=\"{}\"", truncate(&summary, 96)))
        .unwrap_or_default();
    format!("{id} display=\"{}\"{summary}", truncate(&display, 48))
}

fn render_group(value: &Value) -> String {
    let id = field(value, "id").unwrap_or_else(|| "<unknown>".into());
    let name = field(value, "name").unwrap_or_else(|| "unnamed".into());
    let member_count = value
        .get("members")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    format!(
        "{id} name=\"{}\" members={member_count}",
        truncate(&name, 48)
    )
}

fn render_memory_subject(value: &Value) -> String {
    let subject_type = field(value, "subject_type").unwrap_or_else(|| "unknown".into());
    let subject_id = field(value, "subject_id").unwrap_or_else(|| "<unknown>".into());
    let count = field(value, "memory_count").unwrap_or_else(|| "0".into());
    let latest = value
        .get("latest_memory_ids")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(scalar).collect::<Vec<_>>().join(", "))
        .filter(|ids| !ids.is_empty())
        .map(|ids| format!(" latest=[{}]", truncate(&ids, 96)))
        .unwrap_or_default();
    format!("{subject_type}:{subject_id} memories={count}{latest}")
}

fn render_memory(value: &Value) -> String {
    let id = field(value, "id").unwrap_or_else(|| "<unknown>".into());
    let kind = field(value, "memory_type")
        .or_else(|| field(value, "kind"))
        .unwrap_or_else(|| "memory".into());
    let truth = field(value, "truth_status").unwrap_or_else(|| "unknown".into());
    let importance = field(value, "importance").unwrap_or_else(|| "-".into());
    let content = field(value, "content").unwrap_or_default();
    format!(
        "{id} {kind}/{truth} importance={importance} \"{}\"",
        truncate(&content, 120)
    )
}

fn render_intent(value: &Value) -> String {
    let id = field(value, "id").unwrap_or_else(|| "<unknown>".into());
    let status = field(value, "status").unwrap_or_else(|| "unknown".into());
    let fire_at = field(value, "fire_at").unwrap_or_else(|| "-".into());
    let owner_approved = field(value, "owner_approved").unwrap_or_else(|| "false".into());
    let task = field(value, "task").unwrap_or_default();
    format!(
        "{id} status={status} fire_at={fire_at} owner_approved={owner_approved} \"{}\"",
        truncate(&task, 120)
    )
}

fn render_review_job(value: &Value) -> String {
    let source = field(value, "source_action_id").unwrap_or_else(|| "<unknown>".into());
    let review = field(value, "review_action_id").unwrap_or_else(|| "<unknown>".into());
    let review_status = field(value, "review_status").unwrap_or_else(|| "pending".into());
    let outputs = field(value, "output_count").unwrap_or_else(|| "0".into());
    format!("{source} -> {review} status={review_status} outputs={outputs}")
}

fn render_action_trace(value: &Value) -> String {
    let run = value.get("run").unwrap_or(value);
    let id = field(run, "action_id").unwrap_or_else(|| "<unknown>".into());
    let kind = field(run, "kind").unwrap_or_else(|| "unknown".into());
    let status = field(run, "status").unwrap_or_else(|| "unknown".into());
    let responded = field(run, "responded").unwrap_or_else(|| "false".into());
    let turns = len_field(value, "turns");
    let tools = len_field(value, "tool_calls");
    let messages = len_field(value, "messages");
    let deliveries = len_field(value, "deliveries");
    format!(
        "{id} kind={kind} status={status} responded={responded} turns={turns} tools={tools} messages={messages} deliveries={deliveries}"
    )
}

fn render_memory_mutation(value: &Value) -> String {
    let memory = field(value, "memory").unwrap_or_else(|| "<unknown>".into());
    let operation = field(value, "operation").unwrap_or_else(|| "unknown".into());
    let reason = field(value, "reason").unwrap_or_default();
    format!(
        "{memory} operation={operation} reason=\"{}\"",
        truncate(&reason, 96)
    )
}

fn render_failed_event(value: &Value) -> String {
    let id = field(value, "id").unwrap_or_else(|| "<unknown>".into());
    let kind = field(value, "kind").unwrap_or_else(|| "unknown".into());
    let attempts = field(value, "attempts").unwrap_or_else(|| "0".into());
    let error = field(value, "last_error").unwrap_or_default();
    format!(
        "{id} kind={kind} attempts={attempts} error=\"{}\"",
        truncate(&error, 96)
    )
}

fn compact_pairs(value: &Value, pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, label)| {
            let value = field(value, key).unwrap_or_else(|| "-".into());
            format!("{label}={value}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tool_metrics(value: &Value) -> Option<String> {
    let tools = value.get("tool_calls")?.as_object()?;
    if tools.is_empty() {
        return None;
    }

    let mut items = tools
        .iter()
        .filter_map(|(name, counts)| {
            let success = counts.get("success").and_then(Value::as_u64).unwrap_or(0);
            let failure = counts.get("failure").and_then(Value::as_u64).unwrap_or(0);
            (success > 0 || failure > 0).then(|| format!("{name}=ok:{success}/err:{failure}"))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return None;
    }

    items.sort();
    Some(items.join(" "))
}

fn average(value: &Value, numerator: &str, denominator: &str) -> Option<f64> {
    let numerator = value.get(numerator)?.as_f64()?;
    let denominator = value.get(denominator)?.as_f64()?;
    (denominator > 0.0).then_some(numerator / denominator)
}

fn len_field(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(scalar)
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut truncated = value.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests;
