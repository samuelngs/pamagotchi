use crate::id::{ConversationId, GroupId, IdentityId, PersonId, ProfileId};
use crate::media::MediaAttachment;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboundMessage {
    pub message_id: String,
    pub gateway_id: String,
    pub sender_external_id: String,
    pub sender_display_name: Option<String>,
    pub reply_external_id: String,
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
    pub fn sender_key(&self) -> Option<(&str, &str)> {
        if self.gateway_id.is_empty() || self.sender_external_id.is_empty() {
            None
        } else {
            Some((&self.gateway_id, &self.sender_external_id))
        }
    }

    pub fn reply_target(&self) -> Option<(&str, &str)> {
        if self.gateway_id.is_empty() || self.reply_external_id.is_empty() {
            None
        } else {
            Some((&self.gateway_id, &self.reply_external_id))
        }
    }

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
mod tests;
