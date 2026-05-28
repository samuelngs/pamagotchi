use super::*;

#[test]
fn content_kind_names_include_realtime_modes() {
    assert_eq!(GatewayContentKind::Voice.as_str(), "voice");
    assert_eq!(GatewayContentKind::Voice.label(), "Voice");
    assert_eq!(GatewayContentKind::VideoChat.as_str(), "video_chat");
    assert_eq!(GatewayContentKind::VideoChat.label(), "Video chat");
}

#[test]
fn text_only_supports_text_in_both_directions() {
    let capabilities = GatewayContentCapabilities::text_only();

    assert_eq!(capabilities.receive, vec![GatewayContentKind::Text]);
    assert_eq!(capabilities.send, vec![GatewayContentKind::Text]);
}
