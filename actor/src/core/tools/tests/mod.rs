use super::*;

#[test]
fn internal_tools_cannot_send_visible_messages() {
    for kind in [
        ActionKind::Review,
        ActionKind::Research,
        ActionKind::Consolidate,
        ActionKind::Ruminate,
    ] {
        let names = action_tools(&kind)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&"read_messages".to_string()));
        assert!(names.contains(&"update_conversation_summary".to_string()));
        assert!(!names.contains(&"send_message".to_string()));
    }

    let review_names = action_tools(&ActionKind::Review)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert!(review_names.contains(&"upsert_social_relation".to_string()));
    assert!(review_names.contains(&"apply_review".to_string()));
}

#[test]
fn visible_response_tools_can_send_messages() {
    for kind in [ActionKind::Respond, ActionKind::Outreach] {
        let names = action_tools(&kind)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&"send_message".to_string()));
    }
}

#[test]
fn mind_tools_match_mind_prompt_read_only_context_tools() {
    let names = mind_tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert!(names.contains(&"recall_memories".to_string()));
    assert!(names.contains(&"read_messages".to_string()));
    assert!(names.contains(&"get_current_time".to_string()));
    assert!(names.contains(&"respond".to_string()));
    assert!(names.contains(&"drop".to_string()));
    assert!(names.contains(&"defer".to_string()));
    assert!(!names.contains(&"send_message".to_string()));
}
