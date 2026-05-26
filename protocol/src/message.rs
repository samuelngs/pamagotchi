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
    pub media: Option<MediaAttachment>,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

impl InboundMessage {
    pub fn display_content(&self) -> String {
        match &self.media {
            None => self.content.clone(),
            Some(media) => {
                let label = media.kind.label();
                match &media.filename {
                    Some(fname) if self.content.is_empty() => format!("[{label}: {fname}]"),
                    Some(fname) => format!("[{label}: {fname}] {}", self.content),
                    None if self.content.is_empty() => format!("[{label}]"),
                    None => format!("[{label}] {}", self.content),
                }
            }
        }
    }
}
