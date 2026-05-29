use crate::id::{
    ChannelId, ConversationId, GatewayId, GroupId, IdentityId, MessageId, PersonId, ProfileId,
    channel_id,
};
use crate::media::MediaAttachment;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Direct,
    GroupChat,
    PublicChannel,
    PrivateChannel,
    Thread,
    RelayRoom,
    Unknown,
}

impl ChannelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::GroupChat => "group_chat",
            Self::PublicChannel => "public_channel",
            Self::PrivateChannel => "private_channel",
            Self::Thread => "thread",
            Self::RelayRoom => "relay_room",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "group_chat" => Some(Self::GroupChat),
            "public_channel" => Some(Self::PublicChannel),
            "private_channel" => Some(Self::PrivateChannel),
            "thread" => Some(Self::Thread),
            "relay_room" => Some(Self::RelayRoom),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
    DiscordGuild,
    Workspace,
    WhatsAppCommunity,
    Unknown,
}

impl SpaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DiscordGuild => "discord_guild",
            Self::Workspace => "workspace",
            Self::WhatsAppCommunity => "whatsapp_community",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "discord_guild" => Some(Self::DiscordGuild),
            "workspace" => Some(Self::Workspace),
            "whatsapp_community" => Some(Self::WhatsAppCommunity),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    Inbound,
    Outbound,
    Internal,
}

impl MessageDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::Internal => "internal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "inbound" => Some(Self::Inbound),
            "outbound" => Some(Self::Outbound),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Actor,
    System,
    Tool,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Actor => "actor",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "actor" | "assistant" => Some(Self::Actor),
            "system" => Some(Self::System),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboundEnvelope {
    pub gateway_id: GatewayId,
    pub platform_message_id: String,
    pub channel: ChannelKey,
    pub sender: Option<ObservedSender>,
    pub content: String,
    pub attachments: Vec<MediaAttachment>,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

impl InboundEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        if self.platform_message_id.trim().is_empty() {
            return Err("platform_message_id is required".into());
        }
        require_gateway_match("channel", &self.gateway_id, &self.channel.gateway_id)?;
        if let Some(space) = &self.channel.space {
            require_gateway_match("channel.space", &self.gateway_id, &space.gateway_id)?;
        }
        if let Some(parent) = &self.channel.parent {
            require_gateway_match("channel.parent", &self.gateway_id, &parent.gateway_id)?;
            if let Some(parent_space) = &parent.space {
                require_gateway_match(
                    "channel.parent.space",
                    &self.gateway_id,
                    &parent_space.gateway_id,
                )?;
                if let Some(channel_space) = &self.channel.space {
                    if channel_space.external_id != parent_space.external_id {
                        return Err("channel parent crosses spaces".into());
                    }
                }
            }
        }
        if let Some(sender) = &self.sender {
            require_gateway_match(
                "sender.primary",
                &self.gateway_id,
                &sender.primary.gateway_id,
            )?;
            for alias in &sender.aliases {
                require_gateway_match("sender.alias", &self.gateway_id, &alias.gateway_id)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelKey {
    pub gateway_id: GatewayId,
    pub external_id: String,
    pub kind: ChannelKind,
    pub display_name: Option<String>,
    pub space: Option<SpaceKey>,
    pub parent: Option<ParentChannelKey>,
    pub metadata: serde_json::Value,
}

impl ChannelKey {
    pub fn new(
        gateway_id: impl Into<String>,
        external_id: impl Into<String>,
        kind: ChannelKind,
    ) -> Self {
        Self {
            gateway_id: GatewayId(gateway_id.into()),
            external_id: external_id.into(),
            kind,
            display_name: None,
            space: None,
            parent: None,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParentChannelKey {
    pub gateway_id: GatewayId,
    pub external_id: String,
    pub kind: ChannelKind,
    pub display_name: Option<String>,
    pub space: Option<SpaceKey>,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpaceKey {
    pub gateway_id: GatewayId,
    pub external_id: String,
    pub kind: SpaceKind,
    pub display_name: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedSender {
    pub primary: ObservedIdentityKey,
    pub aliases: Vec<ObservedIdentityKey>,
    pub display_name: Option<String>,
    pub metadata: serde_json::Value,
}

impl ObservedSender {
    pub fn primary(
        gateway_id: impl Into<String>,
        external_id: impl Into<String>,
        display_name: Option<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            primary: ObservedIdentityKey {
                gateway_id: GatewayId(gateway_id.into()),
                external_id: external_id.into(),
                kind: None,
                confidence: 1.0,
                source: source.into(),
            },
            aliases: Vec::new(),
            display_name,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedIdentityKey {
    pub gateway_id: GatewayId,
    pub external_id: String,
    pub kind: Option<String>,
    pub confidence: f32,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedInboundMessage {
    pub message_id: MessageId,
    pub platform_message_id: String,
    pub gateway_id: GatewayId,
    pub channel: ChannelId,
    pub conversation: ConversationId,
    pub sender_identity: Option<IdentityId>,
    pub sender_profile: Option<ProfileId>,
    pub sender_person: Option<PersonId>,
    pub sender_display_name: Option<String>,
    pub content: String,
    pub attachments: Vec<MediaAttachment>,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

impl ResolvedInboundMessage {
    pub fn source_key(&self) -> Option<String> {
        if self.gateway_id.0.is_empty() || self.platform_message_id.is_empty() {
            None
        } else {
            Some(format!(
                "{}:{}",
                self.gateway_id.0, self.platform_message_id
            ))
        }
    }

    pub fn channel_target(&self) -> (&GatewayId, &ChannelId) {
        (&self.gateway_id, &self.channel)
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboundMessage {
    pub message_id: String,
    pub gateway_id: String,
    pub sender: Option<ObservedSender>,
    pub channel: ChannelKey,
    pub conversation: ConversationId,
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
        let sender = self.sender.as_ref()?;
        if sender.primary.gateway_id.0.is_empty() || sender.primary.external_id.is_empty() {
            None
        } else {
            Some((&sender.primary.gateway_id.0, &sender.primary.external_id))
        }
    }

    pub fn sender_external_id(&self) -> Option<&str> {
        self.sender_key().map(|(_, external_id)| external_id)
    }

    pub fn sender_display_name(&self) -> Option<&str> {
        self.sender
            .as_ref()
            .and_then(|sender| sender.display_name.as_deref())
    }

    pub fn reply_target(&self) -> Option<(&str, &str)> {
        if self.channel.gateway_id.0.is_empty() || self.channel.external_id.is_empty() {
            None
        } else {
            Some((&self.channel.gateway_id.0, &self.channel.external_id))
        }
    }

    pub fn channel_external_id(&self) -> &str {
        self.channel.external_id.as_str()
    }

    pub fn channel_id(&self) -> ChannelId {
        channel_id(&self.channel.gateway_id, self.channel.external_id.as_str())
    }

    pub fn legacy_group_id(&self) -> Option<GroupId> {
        if let Some(group_id) = self
            .metadata
            .get("group_id")
            .or_else(|| self.metadata.get("guild_id"))
            .and_then(|value| value.as_str())
            .filter(|id| !id.trim().is_empty())
        {
            return Some(GroupId(group_id.to_string()));
        }
        if let Some(group_id) = self
            .channel
            .metadata
            .get("group_id")
            .or_else(|| self.channel.metadata.get("guild_id"))
            .and_then(|value| value.as_str())
            .filter(|id| !id.trim().is_empty())
        {
            return Some(GroupId(group_id.to_string()));
        }
        matches!(self.channel.kind, ChannelKind::GroupChat)
            .then(|| GroupId(self.channel.external_id.clone()))
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

fn require_gateway_match(
    label: &str,
    expected: &GatewayId,
    actual: &GatewayId,
) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "{label} gateway_id mismatch: expected {}, got {}",
            expected.0, actual.0
        ))
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
