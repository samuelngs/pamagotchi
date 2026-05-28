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
