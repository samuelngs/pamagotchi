use super::{clamp_relationship_delta, memory_ids_from_args, thought_memory_ids, tools};
use crate::core::tools::SessionState;
use crate::state::Delta;
use protocol::MemoryId;
use serde_json::json;

#[test]
fn reflect_comm_style_owns_style_and_addressing_preferences() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "reflect")
        .expect("reflect tool exists");
    let description = tool.parameters["properties"]["comm_style"]["description"]
        .as_str()
        .expect("comm_style description exists");

    assert!(description.contains("preferred address"));
    assert!(description.contains("summaries may stay rich"));
    assert!(description.contains("user states a preference"));
}

#[test]
fn note_thought_schema_exposes_quality_metadata() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "note_thought")
        .expect("note_thought tool exists");
    let properties = tool.parameters["properties"]
        .as_object()
        .expect("properties object");

    assert!(properties.contains_key("importance"));
    assert!(properties.contains_key("confidence"));
    assert!(properties.contains_key("memory_ids"));
}

#[test]
fn note_thought_memory_ids_are_parsed() {
    let ids = memory_ids_from_args(&json!({
        "memory_ids": ["memory-a", "", " memory-b "]
    }));
    let ids = ids.into_iter().map(|id| id.0).collect::<Vec<_>>();
    assert_eq!(ids, vec!["memory-a", "memory-b"]);
}

#[test]
fn note_thought_uses_recalled_memory_ids_when_explicit_ids_absent() {
    let state = SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: Delta::default(),
        thoughts: vec![],
        memories_formed: vec![],
        recalled_memory_ids: vec![MemoryId("memory-from-recall".into())],
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

    let ids = thought_memory_ids(&json!({}), &state);
    assert_eq!(ids, vec![MemoryId("memory-from-recall".into())]);

    let explicit = thought_memory_ids(
        &json!({
            "memory_ids": ["memory-explicit"]
        }),
        &state,
    );
    assert_eq!(explicit, vec![MemoryId("memory-explicit".into())]);
}

#[test]
fn relationship_deltas_are_small_per_reflection() {
    assert_eq!(clamp_relationship_delta(1.0, 0.05), 0.05);
    assert_eq!(clamp_relationship_delta(-1.0, 0.05), -0.05);
    assert_eq!(clamp_relationship_delta(0.02, 0.05), 0.02);
}
