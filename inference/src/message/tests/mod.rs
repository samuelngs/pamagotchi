use super::*;

#[test]
fn malformed_tool_arguments_are_preserved_as_error_marker() {
    let arguments = parse_tool_arguments("{\"content\":");

    assert_eq!(arguments["__invalid_tool_json"], true);
    assert_eq!(arguments["raw_arguments"], "{\"content\":");
    assert!(
        arguments["error"]
            .as_str()
            .is_some_and(|error| { error.contains("EOF") || error.contains("expected") })
    );
}
