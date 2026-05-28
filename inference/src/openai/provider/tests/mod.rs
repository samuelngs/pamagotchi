use super::*;

#[test]
fn non_streaming_tool_call_preserves_malformed_arguments() {
    let call = wire_response_tool_call(WireResponseToolCall {
        id: "call-1".into(),
        function: WireResponseFunction {
            name: "send_message".into(),
            arguments: "{\"content\":".into(),
        },
    });

    assert_eq!(call.id, "call-1");
    assert_eq!(call.name, "send_message");
    assert_eq!(call.arguments["__invalid_tool_json"], true);
    assert_eq!(call.arguments["raw_arguments"], "{\"content\":");
    assert!(
        call.arguments["error"]
            .as_str()
            .is_some_and(|error| { error.contains("EOF") || error.contains("expected") })
    );
}
