use super::*;
use crate::{ConversationId, MediaAssetId, MediaAttachment, MediaKind};

fn inbound(attachments: Vec<MediaAttachment>, content: &str) -> InboundMessage {
    InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "whatsapp".into(),
        sender: Some(ObservedSender {
            primary: ObservedIdentityKey {
                gateway_id: GatewayId("whatsapp".into()),
                external_id: "sender-1".into(),
                kind: None,
                confidence: 1.0,
                source: "test".into(),
            },
            aliases: Vec::new(),
            display_name: Some("Sender".into()),
            metadata: serde_json::Value::Null,
        }),
        channel: ChannelKey {
            gateway_id: GatewayId("whatsapp".into()),
            external_id: "chat-1".into(),
            kind: ChannelKind::Direct,
            display_name: None,
            space: None,
            parent: None,
            metadata: serde_json::Value::Null,
        },
        conversation: ConversationId("whatsapp:chat-1".into()),
        identity: None,
        profile: None,
        person: None,
        content: content.into(),
        attachments,
        timestamp: 1,
        metadata: serde_json::Value::Null,
    }
}

#[test]
fn display_content_includes_media_asset_details() {
    let msg = inbound(
        vec![MediaAttachment {
            kind: MediaKind::Image,
            asset_id: Some(MediaAssetId("media-123".into())),
            url: None,
            mime: Some("image/png".into()),
            filename: Some("photo.png".into()),
            size: Some(42),
        }],
        "caption",
    );

    assert_eq!(
        msg.display_content(),
        "[Image: kind=image asset_id=media-123 filename=photo.png mime=image/png size=42] caption"
    );
}

#[test]
fn display_content_includes_multiple_attachments() {
    let msg = inbound(
        vec![
            MediaAttachment {
                kind: MediaKind::Image,
                asset_id: Some(MediaAssetId("image-1".into())),
                url: None,
                mime: Some("image/png".into()),
                filename: Some("photo.png".into()),
                size: None,
            },
            MediaAttachment {
                kind: MediaKind::File,
                asset_id: Some(MediaAssetId("file-1".into())),
                url: None,
                mime: Some("application/pdf".into()),
                filename: Some("doc.pdf".into()),
                size: Some(100),
            },
        ],
        "",
    );

    assert_eq!(
        msg.display_content(),
        "[Image: kind=image asset_id=image-1 filename=photo.png mime=image/png] [File: kind=file asset_id=file-1 filename=doc.pdf mime=application/pdf size=100]"
    );
}

#[test]
fn display_content_includes_media_only_audio_and_sticker() {
    let msg = inbound(
        vec![
            MediaAttachment {
                kind: MediaKind::Audio,
                asset_id: Some(MediaAssetId("voice-1".into())),
                url: None,
                mime: Some("audio/ogg".into()),
                filename: None,
                size: Some(25),
            },
            MediaAttachment {
                kind: MediaKind::Sticker,
                asset_id: Some(MediaAssetId("sticker-1".into())),
                url: None,
                mime: Some("image/webp".into()),
                filename: Some("sticker.webp".into()),
                size: None,
            },
        ],
        "",
    );

    assert_eq!(
        msg.display_content(),
        "[Audio: kind=audio asset_id=voice-1 mime=audio/ogg size=25] [Sticker: kind=sticker asset_id=sticker-1 filename=sticker.webp mime=image/webp]"
    );
}

#[test]
fn sender_key_and_reply_target_are_independent() {
    let msg = inbound(Vec::new(), "hello");

    assert_eq!(msg.sender_key(), Some(("whatsapp", "sender-1")));
    assert_eq!(msg.reply_target(), Some(("whatsapp", "chat-1")));
}
