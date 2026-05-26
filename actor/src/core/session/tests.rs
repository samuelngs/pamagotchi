use super::messages::{message_metadata, required_capabilities};
use inference::Capability;
use protocol::{ConversationId, InboundMessage, MediaAssetId, MediaAttachment, MediaKind};
use serde_json::Value;

fn inbound(metadata: Value) -> InboundMessage {
    InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "whatsapp".into(),
        external_id: "chat-1".into(),
        conversation: ConversationId("whatsapp:chat-1".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
        content: String::new(),
        attachments: vec![MediaAttachment {
            kind: MediaKind::Sticker,
            asset_id: Some(MediaAssetId("media-1".into())),
            url: None,
            mime: Some("image/webp".into()),
            filename: Some("sticker.webp".into()),
            size: Some(99),
        }],
        timestamp: 1,
        metadata,
    }
}

#[test]
fn message_metadata_embeds_attachments() {
    let metadata = message_metadata(&inbound(serde_json::json!({ "sender": "user" })));

    assert_eq!(metadata["sender"], "user");
    assert_eq!(metadata["attachments"][0]["kind"], "Sticker");
    assert_eq!(metadata["attachments"][0]["asset_id"], "media-1");
    assert_eq!(metadata["attachments"][0]["mime"], "image/webp");
}

#[test]
fn visual_attachments_require_vision() {
    let mut msg = inbound(Value::Null);
    msg.attachments[0].kind = MediaKind::Video;

    assert_eq!(required_capabilities(&[msg], &[]), vec![Capability::Vision]);
}

#[test]
fn file_attachments_do_not_require_vision() {
    let mut msg = inbound(Value::Null);
    msg.attachments[0].kind = MediaKind::File;

    assert!(required_capabilities(&[msg], &[]).is_empty());
}
