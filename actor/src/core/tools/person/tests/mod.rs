use super::tools;

fn tool_description(name: &str, property: &str) -> String {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == name)
        .expect("tool exists");
    tool.parameters["properties"][property]["description"]
        .as_str()
        .expect("property description exists")
        .to_string()
}

#[test]
fn update_person_summary_allows_rich_summary_with_separate_style() {
    let description = tool_description("update_person", "summary");

    assert!(description.contains("Rich person-level summary"));
    assert!(description.contains("comm_style"));
    assert!(!description.contains("non-style"));
}

#[test]
fn update_profile_can_write_summary_and_style() {
    let summary = tool_description("update_profile", "summary");
    let style = tool_description("update_profile", "comm_style");

    assert!(summary.contains("account-specific summary"));
    assert!(style.contains("Communication style"));
}

#[test]
fn get_person_identity_lookup_has_reason_field() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "get_person")
        .expect("get_person exists");
    let properties = &tool.parameters["properties"];

    assert!(properties.get("include_identities").is_some());
    assert!(properties.get("reason").is_some());
    assert!(properties.get("delivery_required").is_some());
    assert!(
        properties["reason"]["description"]
            .as_str()
            .unwrap()
            .contains("Required when include_identities=true")
    );
}

#[test]
fn social_relation_tool_exposes_evidence_metadata() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "upsert_social_relation")
        .expect("upsert_social_relation exists");
    let properties = &tool.parameters["properties"];

    assert!(properties.get("confidence").is_some());
    assert!(properties.get("direction").is_some());
    assert!(properties.get("status").is_some());
    assert!(properties.get("source_kind").is_some());
    assert!(properties.get("asserted_by_person_id").is_some());
    assert!(properties.get("evidence").is_some());
    assert!(properties.get("evidence_message_ids").is_some());
    assert!(properties.get("evidence_quote").is_some());
}
