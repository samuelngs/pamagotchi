use super::tools;

#[test]
fn form_memory_exposes_existing_structured_fields() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "form_memory")
        .expect("form_memory exists");
    let properties = &tool.parameters["properties"];

    assert!(properties.get("tags").is_some());
    assert!(properties.get("sensitivity").is_some());
    assert!(properties.get("emotional_valence").is_some());
    assert!(properties.get("memory_type").is_some());
    assert!(properties.get("truth_status").is_some());
    assert!(properties.get("confidence").is_some());
    assert!(properties.get("evidence_message_ids").is_some());
    assert!(properties.get("source_spans").is_some());
    assert!(properties.get("evidence").is_some());
    assert!(properties.get("dedupe_key").is_some());
    assert!(properties.get("subject_actor").is_some());
    assert!(properties.get("supersedes").is_some());
    assert!(properties.get("contradiction_group").is_some());
    assert!(properties.get("last_confirmed_at").is_some());
    assert!(properties.get("next_review_at").is_some());
    assert!(properties.get("include_sensitive").is_none());
}

#[test]
fn recall_memory_schema_exposes_sensitive_opt_in() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "recall_memories")
        .expect("recall_memories exists");
    let properties = &tool.parameters["properties"];

    assert!(properties.get("max_sensitivity").is_some());
    assert!(properties.get("include_sensitive").is_some());
    assert!(properties.get("include_superseded").is_some());
}

#[test]
fn memory_tools_expose_chosen_person_inspection_and_deletion() {
    let tools = tools();
    let names = tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();

    assert!(names.contains(&"inspect_memory".to_string()));
    assert!(names.contains(&"delete_memory".to_string()));
}
