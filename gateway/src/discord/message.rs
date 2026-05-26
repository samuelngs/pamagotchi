use super::DiscordConfig;
use crate::{GatewayConnectionState, GatewayRuntime};
use async_trait::async_trait;
use protocol::{ConversationId, GroupId, InboundMessage, MediaAttachment, MediaKind};
use serenity::all::{ChannelId, Context, EventHandler, Message, Ready};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct DiscordHandler {
    pub gateway_id: String,
    pub config: DiscordConfig,
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    pub runtime: Arc<GatewayRuntime>,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(
            gateway = %self.gateway_id,
            user = %ready.user.name,
            "discord gateway connected"
        );
        self.runtime
            .emit_state(&self.gateway_id, GatewayConnectionState::Connected)
            .await;
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        if self.config.ignore_bots && msg.author.bot {
            return;
        }

        let channel_id = msg.channel_id.get();
        if !self.config.allows_channel(channel_id) {
            return;
        }

        let (content, attachments) = extract_message_content(&msg);
        if content.trim().is_empty() && attachments.is_empty() {
            return;
        }

        let inbound = InboundMessage {
            message_id: msg.id.to_string(),
            gateway_id: self.gateway_id.clone(),
            external_id: channel_id.to_string(),
            conversation: ConversationId(format!("{}:{channel_id}", self.gateway_id)),
            group: msg
                .guild_id
                .map(|guild_id| GroupId(format!("discord:{}", guild_id.get()))),
            identity: None,
            profile: None,
            person: None,
            content,
            attachments,
            timestamp: msg.timestamp.unix_timestamp(),
            metadata: serde_json::json!({
                "platform": "discord",
                "channel_id": channel_id.to_string(),
                "guild_id": msg.guild_id.map(|guild_id| guild_id.get().to_string()),
                "author_id": msg.author.id.get().to_string(),
                "author_name": msg.author.name,
                "message_id": msg.id.get().to_string(),
            }),
        };

        if let Err(e) = self.inbound_tx.send(inbound).await {
            warn!(%e, gateway = %self.gateway_id, "failed to forward discord message");
        }
    }
}

fn extract_message_content(msg: &Message) -> (String, Vec<MediaAttachment>) {
    let attachments = msg
        .attachments
        .iter()
        .map(|attachment| {
            discord_attachment(
                attachment.url.clone(),
                attachment.content_type.clone(),
                attachment.filename.clone(),
                u64::from(attachment.size),
            )
        })
        .collect();

    (msg.content.clone(), attachments)
}

pub(super) fn discord_attachment(
    url: String,
    content_type: Option<String>,
    filename: String,
    size: u64,
) -> MediaAttachment {
    MediaAttachment {
        kind: media_kind_from_mime(content_type.as_deref()),
        asset_id: None,
        url: Some(url),
        mime: content_type,
        filename: Some(filename),
        size: Some(size),
    }
}

pub(super) fn media_kind_from_mime(mime: Option<&str>) -> MediaKind {
    match mime.unwrap_or_default() {
        mime if mime.starts_with("image/") => MediaKind::Image,
        mime if mime.starts_with("video/") => MediaKind::Video,
        mime if mime.starts_with("audio/") => MediaKind::Audio,
        _ => MediaKind::File,
    }
}

pub fn parse_channel_id(external_id: &str) -> anyhow::Result<ChannelId> {
    let id = external_id
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid Discord channel id: {external_id}"))?;
    Ok(ChannelId::new(id))
}
