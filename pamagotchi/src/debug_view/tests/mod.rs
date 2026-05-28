use super::*;

#[test]
fn formats_debug_snapshot_into_named_sections() {
    let snapshot = serde_json::json!({
        "generated_at": 1000,
        "limit": 10,
        "metrics": {
            "events_received": 3,
            "events_dropped": 1,
            "events_deferred": 2,
            "event_queue_depth": 0,
            "actions_spawned": 4,
            "actions_completed": 3,
            "actions_failed": 1,
            "running_actions": 0,
            "memory_created": 2,
            "memory_updated": 1,
            "memory_superseded": 0,
            "review_outputs": 2,
            "outbound_delivery_success": 1,
            "outbound_delivery_failure": 0,
            "injection_successes": 1,
            "injection_failures": 0,
            "malformed_tool_json": 2,
            "malformed_tool_json_by_model": {"gpt-test": 2},
            "review_latency_ms_total": 50,
            "prompt_turns_with_usage": 2,
            "prompt_input_tokens": 120,
            "prompt_output_tokens": 30,
            "recall_latency_ms_total": 20,
            "recall_result_count": 6,
            "recall_calls": 2,
            "app_server_tool_calls": 2,
            "app_server_tool_latency_ms_total": 70,
            "tool_calls": {
                "send_message": {"success": 1, "failure": 0},
                "recall_memories": {"success": 1, "failure": 1}
            }
        },
        "persons": [{"id": "person-1", "name": "Sam"}],
        "profiles": [{"id": "profile-1", "display_name": "Sam Relay"}],
        "groups": [{"id": "group-1", "name": "Friends", "members": ["person-1"]}],
        "memory_subjects": [{"subject_type": "person", "subject_id": "person-1", "memory_count": 2, "latest_memory_ids": ["memory-1"]}],
        "memories": [{"id": "memory-1", "memory_type": "fact", "truth_status": "observed", "importance": 0.7, "content": "Sam likes concise status updates."}],
        "intents": [{"id": "intent-1", "status": "active", "fire_at": 1200, "chosen_person_approved": true, "task": "Follow up"}],
        "review_jobs": [{"source_action_id": "action-1", "review_action_id": "review-1", "review_status": "completed", "output_count": 3}],
        "action_traces": [{"run": {"action_id": "action-1", "kind": "respond", "status": "completed", "responded": true}, "turns": [{}], "tool_calls": [{}, {}], "messages": [{}], "deliveries": []}],
        "memory_mutations": [{"memory": "memory-1", "operation": "create", "reason": "review"}],
        "failed_events": [{"id": 1, "kind": "message", "attempts": 2, "last_error": "bad payload"}]
    });

    let output = format_snapshot(&snapshot);

    assert!(output.contains("Debug snapshot"));
    assert!(output.contains("Persons (1)"));
    assert!(output.contains("Memories by subject (1)"));
    assert!(output.contains("Review jobs (1)"));
    assert!(output.contains("Action traces (1)"));
    assert!(output.contains("prompt usage: turns=2 input_tokens=120 output_tokens=30"));
    assert!(output.contains("app-server tools: calls=2 latency_ms=70"));
    assert!(output.contains("tool calls: recall_memories=ok:1/err:1 send_message=ok:1/err:0"));
    assert!(output.contains("avg_prompt_input_tokens: 60.0"));
    assert!(output.contains("avg_prompt_output_tokens: 15.0"));
    assert!(output.contains("avg_recall_results: 3.0"));
    assert!(output.contains("avg_app_server_tool_latency_ms: 35.0"));
    assert!(output.contains("malformed by model: gpt-test=2"));
}
