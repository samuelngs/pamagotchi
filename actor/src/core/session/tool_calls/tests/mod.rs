use super::*;

#[test]
fn malformed_tool_arguments_are_preserved_as_error() {
    let calls = finalize_tool_calls(vec![PartialToolCall {
        id: "call-1".into(),
        name: "send_message".into(),
        arguments: "{\"content\":".into(),
    }]);

    assert_eq!(calls.len(), 1);
    let error = invalid_tool_json_error(&calls[0].arguments).expect("invalid json marker");
    assert!(error.contains("EOF") || error.contains("expected"));
    assert_eq!(calls[0].arguments["raw_arguments"], "{\"content\":");
}
