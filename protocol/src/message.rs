use crate::id::{ConversationId, GroupId, IdentityId, PersonId, ProfileId};
use crate::media::MediaAttachment;

#[derive(Clone, Debug)]
pub struct InboundMessage {
    pub message_id: String,
    pub gateway_id: String,
    pub external_id: String,
    pub conversation: ConversationId,
    pub group: Option<GroupId>,
    pub identity: Option<IdentityId>,
    pub profile: Option<ProfileId>,
    pub person: Option<PersonId>,
    pub content: String,
    pub attachments: Vec<MediaAttachment>,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

impl InboundMessage {
    pub fn display_content(&self) -> String {
        if self.attachments.is_empty() {
            return self.content.clone();
        }

        let mut parts: Vec<String> = self.attachments.iter().map(describe_attachment).collect();
        if !self.content.is_empty() {
            parts.push(self.content.clone());
        }
        parts.join(" ")
    }
}

fn describe_attachment(media: &MediaAttachment) -> String {
    let label = media.kind.label();
    let mut parts = vec![format!("kind={}", media.kind.as_str())];
    if let Some(asset_id) = &media.asset_id {
        parts.push(format!("asset_id={}", asset_id.0));
    }
    if let Some(filename) = &media.filename {
        parts.push(format!("filename={filename}"));
    }
    if let Some(mime) = &media.mime {
        parts.push(format!("mime={mime}"));
    }
    if let Some(size) = media.size {
        parts.push(format!("size={size}"));
    }

    format!("[{label}: {}]", parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaAssetId, MediaAttachment, MediaKind};

    fn inbound(attachments: Vec<MediaAttachment>, content: &str) -> InboundMessage {
        InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "whatsapp".into(),
            external_id: "chat-1".into(),
            conversation: ConversationId("whatsapp:chat-1".into()),
            group: None,
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
}
