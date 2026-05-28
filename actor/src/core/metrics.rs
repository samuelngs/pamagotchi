use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct ActorMetrics {
    events_received: AtomicU64,
    events_dropped: AtomicU64,
    events_deferred: AtomicU64,
    event_queue_depth: AtomicU64,
    action_queue_length: AtomicU64,
    running_actions: AtomicU64,
    retained_completed_actions: AtomicU64,
    actions_spawned: AtomicU64,
    actions_completed: AtomicU64,
    actions_failed: AtomicU64,
    actions_cancelled: AtomicU64,
    injection_successes: AtomicU64,
    injection_failures: AtomicU64,
    duplicate_message_suppressions: AtomicU64,
    memory_created: AtomicU64,
    memory_updated: AtomicU64,
    memory_superseded: AtomicU64,
    memories_pruned: AtomicU64,
    thoughts_pruned: AtomicU64,
    review_outputs: AtomicU64,
    review_latency_ms_total: AtomicU64,
    outbound_delivery_success: AtomicU64,
    outbound_delivery_failure: AtomicU64,
    prompt_turns_with_usage: AtomicU64,
    prompt_input_tokens: AtomicU64,
    prompt_output_tokens: AtomicU64,
    recall_calls: AtomicU64,
    recall_latency_ms_total: AtomicU64,
    recall_result_count: AtomicU64,
    app_server_tool_calls: AtomicU64,
    app_server_tool_latency_ms_total: AtomicU64,
    malformed_tool_json: AtomicU64,
    malformed_tool_json_by_model: Mutex<BTreeMap<String, u64>>,
    tool_calls: Mutex<BTreeMap<String, ToolCallMetricsSnapshot>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ActorMetricsSnapshot {
    pub events_received: u64,
    pub events_dropped: u64,
    pub events_deferred: u64,
    pub event_queue_depth: u64,
    pub action_queue_length: u64,
    pub running_actions: u64,
    pub retained_completed_actions: u64,
    pub actions_spawned: u64,
    pub actions_completed: u64,
    pub actions_failed: u64,
    pub actions_cancelled: u64,
    pub injection_successes: u64,
    pub injection_failures: u64,
    pub duplicate_message_suppressions: u64,
    pub memory_created: u64,
    pub memory_updated: u64,
    pub memory_superseded: u64,
    pub memories_pruned: u64,
    pub thoughts_pruned: u64,
    pub review_outputs: u64,
    pub review_latency_ms_total: u64,
    pub outbound_delivery_success: u64,
    pub outbound_delivery_failure: u64,
    pub prompt_turns_with_usage: u64,
    pub prompt_input_tokens: u64,
    pub prompt_output_tokens: u64,
    pub recall_calls: u64,
    pub recall_latency_ms_total: u64,
    pub recall_result_count: u64,
    pub app_server_tool_calls: u64,
    pub app_server_tool_latency_ms_total: u64,
    pub malformed_tool_json: u64,
    pub malformed_tool_json_by_model: BTreeMap<String, u64>,
    pub tool_calls: BTreeMap<String, ToolCallMetricsSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ToolCallMetricsSnapshot {
    pub success: u64,
    pub failure: u64,
}

impl ActorMetrics {
    pub fn snapshot(&self) -> ActorMetricsSnapshot {
        ActorMetricsSnapshot {
            events_received: self.load(&self.events_received),
            events_dropped: self.load(&self.events_dropped),
            events_deferred: self.load(&self.events_deferred),
            event_queue_depth: self.load(&self.event_queue_depth),
            action_queue_length: self.load(&self.action_queue_length),
            running_actions: self.load(&self.running_actions),
            retained_completed_actions: self.load(&self.retained_completed_actions),
            actions_spawned: self.load(&self.actions_spawned),
            actions_completed: self.load(&self.actions_completed),
            actions_failed: self.load(&self.actions_failed),
            actions_cancelled: self.load(&self.actions_cancelled),
            injection_successes: self.load(&self.injection_successes),
            injection_failures: self.load(&self.injection_failures),
            duplicate_message_suppressions: self.load(&self.duplicate_message_suppressions),
            memory_created: self.load(&self.memory_created),
            memory_updated: self.load(&self.memory_updated),
            memory_superseded: self.load(&self.memory_superseded),
            memories_pruned: self.load(&self.memories_pruned),
            thoughts_pruned: self.load(&self.thoughts_pruned),
            review_outputs: self.load(&self.review_outputs),
            review_latency_ms_total: self.load(&self.review_latency_ms_total),
            outbound_delivery_success: self.load(&self.outbound_delivery_success),
            outbound_delivery_failure: self.load(&self.outbound_delivery_failure),
            prompt_turns_with_usage: self.load(&self.prompt_turns_with_usage),
            prompt_input_tokens: self.load(&self.prompt_input_tokens),
            prompt_output_tokens: self.load(&self.prompt_output_tokens),
            recall_calls: self.load(&self.recall_calls),
            recall_latency_ms_total: self.load(&self.recall_latency_ms_total),
            recall_result_count: self.load(&self.recall_result_count),
            app_server_tool_calls: self.load(&self.app_server_tool_calls),
            app_server_tool_latency_ms_total: self.load(&self.app_server_tool_latency_ms_total),
            malformed_tool_json: self.load(&self.malformed_tool_json),
            malformed_tool_json_by_model: self.malformed_tool_json_by_model.lock().unwrap().clone(),
            tool_calls: self.tool_calls.lock().unwrap().clone(),
        }
    }

    pub fn record_event_received(&self) {
        self.inc(&self.events_received, 1);
    }

    pub fn record_event_dropped(&self) {
        self.inc(&self.events_dropped, 1);
    }

    pub fn record_event_deferred(&self) {
        self.inc(&self.events_deferred, 1);
    }

    pub fn set_event_queue_depth(&self, depth: u64) {
        self.event_queue_depth.store(depth, Ordering::Relaxed);
    }

    pub fn observe_registry(&self, queued: u64, running: u64, retained_completed: u64) {
        self.action_queue_length.store(queued, Ordering::Relaxed);
        self.running_actions.store(running, Ordering::Relaxed);
        self.retained_completed_actions
            .store(retained_completed, Ordering::Relaxed);
    }

    pub fn record_action_spawned(&self) {
        self.inc(&self.actions_spawned, 1);
    }

    pub fn record_action_completed(&self, failed: bool) {
        self.inc(&self.actions_completed, 1);
        if failed {
            self.inc(&self.actions_failed, 1);
        }
    }

    pub fn record_action_cancelled(&self) {
        self.inc(&self.actions_cancelled, 1);
    }

    pub fn record_injection(&self, success: bool) {
        if success {
            self.inc(&self.injection_successes, 1);
        } else {
            self.inc(&self.injection_failures, 1);
        }
    }

    pub fn record_duplicate_message_suppression(&self) {
        self.inc(&self.duplicate_message_suppressions, 1);
    }

    pub fn record_memory_created(&self) {
        self.inc(&self.memory_created, 1);
    }

    pub fn record_memory_updated(&self) {
        self.inc(&self.memory_updated, 1);
    }

    pub fn record_memory_superseded(&self) {
        self.inc(&self.memory_superseded, 1);
    }

    pub fn record_memories_pruned(&self, count: usize) {
        self.inc(&self.memories_pruned, count as u64);
    }

    pub fn record_thoughts_pruned(&self, count: usize) {
        self.inc(&self.thoughts_pruned, count as u64);
    }

    pub fn record_review_output(&self, latency_ms: Option<u64>) {
        self.inc(&self.review_outputs, 1);
        if let Some(latency_ms) = latency_ms {
            self.inc(&self.review_latency_ms_total, latency_ms);
        }
    }

    pub fn record_outbound_delivery(&self, success: bool) {
        if success {
            self.inc(&self.outbound_delivery_success, 1);
        } else {
            self.inc(&self.outbound_delivery_failure, 1);
        }
    }

    pub fn record_prompt_tokens(&self, input_tokens: u32, output_tokens: u32) {
        self.inc(&self.prompt_turns_with_usage, 1);
        self.inc(&self.prompt_input_tokens, input_tokens as u64);
        self.inc(&self.prompt_output_tokens, output_tokens as u64);
    }

    pub fn record_recall(&self, latency_ms: u64, result_count: usize) {
        self.inc(&self.recall_calls, 1);
        self.inc(&self.recall_latency_ms_total, latency_ms);
        self.inc(&self.recall_result_count, result_count as u64);
    }

    pub fn record_app_server_tool_latency(&self, latency_ms: u64) {
        self.inc(&self.app_server_tool_calls, 1);
        self.inc(&self.app_server_tool_latency_ms_total, latency_ms);
    }

    pub fn record_tool_call(&self, name: &str, success: bool) {
        let mut tools = self.tool_calls.lock().unwrap();
        let entry = tools.entry(name.to_string()).or_default();
        if success {
            entry.success += 1;
        } else {
            entry.failure += 1;
        }
    }

    pub fn record_malformed_tool_json(&self, model: &str) {
        self.inc(&self.malformed_tool_json, 1);
        let key = if model.trim().is_empty() {
            "<unknown>".to_string()
        } else {
            model.to_string()
        };
        let mut by_model = self.malformed_tool_json_by_model.lock().unwrap();
        *by_model.entry(key).or_default() += 1;
    }

    fn inc(&self, counter: &AtomicU64, amount: u64) {
        counter.fetch_add(amount, Ordering::Relaxed);
    }

    fn load(&self, counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_snapshot_tracks_counters_and_tool_names() {
        let metrics = ActorMetrics::default();

        metrics.record_event_received();
        metrics.record_event_dropped();
        metrics.record_event_deferred();
        metrics.set_event_queue_depth(3);
        metrics.observe_registry(2, 1, 4);
        metrics.record_action_spawned();
        metrics.record_action_completed(true);
        metrics.record_action_cancelled();
        metrics.record_injection(true);
        metrics.record_injection(false);
        metrics.record_duplicate_message_suppression();
        metrics.record_memory_created();
        metrics.record_memory_updated();
        metrics.record_memory_superseded();
        metrics.record_memories_pruned(2);
        metrics.record_thoughts_pruned(3);
        metrics.record_review_output(Some(23));
        metrics.record_outbound_delivery(true);
        metrics.record_outbound_delivery(false);
        metrics.record_prompt_tokens(11, 7);
        metrics.record_recall(13, 5);
        metrics.record_app_server_tool_latency(17);
        metrics.record_tool_call("send_message", true);
        metrics.record_tool_call("send_message", false);
        metrics.record_malformed_tool_json("gpt-test");
        metrics.record_malformed_tool_json("gpt-test");

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.events_received, 1);
        assert_eq!(snapshot.events_dropped, 1);
        assert_eq!(snapshot.events_deferred, 1);
        assert_eq!(snapshot.event_queue_depth, 3);
        assert_eq!(snapshot.action_queue_length, 2);
        assert_eq!(snapshot.running_actions, 1);
        assert_eq!(snapshot.retained_completed_actions, 4);
        assert_eq!(snapshot.actions_spawned, 1);
        assert_eq!(snapshot.actions_completed, 1);
        assert_eq!(snapshot.actions_failed, 1);
        assert_eq!(snapshot.actions_cancelled, 1);
        assert_eq!(snapshot.injection_successes, 1);
        assert_eq!(snapshot.injection_failures, 1);
        assert_eq!(snapshot.duplicate_message_suppressions, 1);
        assert_eq!(snapshot.memory_created, 1);
        assert_eq!(snapshot.memory_updated, 1);
        assert_eq!(snapshot.memory_superseded, 1);
        assert_eq!(snapshot.memories_pruned, 2);
        assert_eq!(snapshot.thoughts_pruned, 3);
        assert_eq!(snapshot.review_outputs, 1);
        assert_eq!(snapshot.review_latency_ms_total, 23);
        assert_eq!(snapshot.outbound_delivery_success, 1);
        assert_eq!(snapshot.outbound_delivery_failure, 1);
        assert_eq!(snapshot.prompt_turns_with_usage, 1);
        assert_eq!(snapshot.prompt_input_tokens, 11);
        assert_eq!(snapshot.prompt_output_tokens, 7);
        assert_eq!(snapshot.recall_calls, 1);
        assert_eq!(snapshot.recall_latency_ms_total, 13);
        assert_eq!(snapshot.recall_result_count, 5);
        assert_eq!(snapshot.app_server_tool_calls, 1);
        assert_eq!(snapshot.app_server_tool_latency_ms_total, 17);
        assert_eq!(snapshot.malformed_tool_json, 2);
        assert_eq!(
            snapshot.malformed_tool_json_by_model.get("gpt-test"),
            Some(&2)
        );
        assert_eq!(
            snapshot.tool_calls["send_message"],
            ToolCallMetricsSnapshot {
                success: 1,
                failure: 1,
            }
        );
    }
}
