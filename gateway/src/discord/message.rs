use super::DiscordConfig;
use crate::{GatewayConnectionState, GatewayRuntime};
use async_trait::async_trait;
use protocol::{
    ChannelKey as ProtocolChannelKey, ChannelKind, GatewayId, InboundEnvelope, MediaAttachment,
    MediaKind, ObservedIdentityKey, ObservedSender, ParentChannelKey, SpaceKey, SpaceKind,
};
use serenity::all::{
    ChannelId, Context, EventHandler, Message, MessageId, MessageUpdateEvent, Ready,
    TypingStartEvent,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct DiscordHandler {
    pub gateway_id: String,
    pub config: DiscordConfig,
    pub inbound_tx: mpsc::Sender<InboundEnvelope>,
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

        let gateway = GatewayId(self.gateway_id.clone());
        let channel = discord_channel_key(
            &gateway,
            channel_id,
            msg.guild_id.map(|guild_id| guild_id.get()),
            None,
        );
        let inbound = InboundEnvelope {
            gateway_id: gateway.clone(),
            platform_message_id: msg.id.to_string(),
            channel,
            sender: Some(ObservedSender {
                primary: discord_identity_key(&gateway, msg.author.id.get(), "primary_sender"),
                aliases: vec![],
                display_name: Some(msg.author.name.clone()),
                metadata: serde_json::json!({
                    "author_name": msg.author.name,
                    "bot": msg.author.bot,
                }),
            }),
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

        if let Err(e) = inbound.validate() {
            warn!(%e, gateway = %self.gateway_id, message_id = %msg.id, "invalid discord inbound envelope");
            return;
        }

        if let Err(e) = self.inbound_tx.send(inbound).await {
            warn!(%e, gateway = %self.gateway_id, "failed to forward discord message");
        }
    }

    async fn typing_start(&self, _ctx: Context, event: TypingStartEvent) {
        if self.config.ignore_bots && event.member.as_ref().is_some_and(|member| member.user.bot) {
            return;
        }

        let channel_id = event.channel_id.get();
        if !self.config.allows_channel(channel_id) {
            return;
        }

        let gateway = GatewayId(self.gateway_id.clone());
        self.runtime
            .emit_typing(
                &self.gateway_id,
                discord_channel_key(
                    &gateway,
                    channel_id,
                    event.guild_id.map(|guild_id| guild_id.get()),
                    None,
                ),
                discord_identity_key(&gateway, event.user_id.get(), "typing"),
                true,
            )
            .await;
    }

    async fn message_update(
        &self,
        _ctx: Context,
        _old_if_available: Option<Message>,
        new: Option<Message>,
        event: MessageUpdateEvent,
    ) {
        let channel_id = event.channel_id.get();
        if !self.config.allows_channel(channel_id) {
            return;
        }
        let is_bot_update = new.as_ref().is_some_and(|message| message.author.bot)
            || event.author.as_ref().is_some_and(|author| author.bot);
        if self.config.ignore_bots && is_bot_update {
            return;
        }

        let content = match new.as_ref() {
            Some(message) => extract_message_content(message).0,
            None => match event.content.clone() {
                Some(content) => content,
                None => return,
            },
        };
        let edited_at = event
            .edited_timestamp
            .or(event.timestamp)
            .map(|timestamp| timestamp.unix_timestamp())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        let gateway = GatewayId(self.gateway_id.clone());
        self.runtime
            .emit_message_edited(
                &self.gateway_id,
                discord_channel_key(
                    &gateway,
                    channel_id,
                    event.guild_id.map(|guild_id| guild_id.get()),
                    None,
                ),
                event.id.to_string(),
                content,
                edited_at,
            )
            .await;
    }

    async fn message_delete(
        &self,
        _ctx: Context,
        channel_id: ChannelId,
        deleted_message_id: MessageId,
        _guild_id: Option<serenity::all::GuildId>,
    ) {
        let channel_id_raw = channel_id.get();
        if !self.config.allows_channel(channel_id_raw) {
            return;
        }

        let gateway = GatewayId(self.gateway_id.clone());
        self.runtime
            .emit_message_deleted(
                &self.gateway_id,
                discord_channel_key(
                    &gateway,
                    channel_id_raw,
                    _guild_id.map(|guild_id| guild_id.get()),
                    None,
                ),
                deleted_message_id.to_string(),
                chrono::Utc::now().timestamp(),
            )
            .await;
    }

    async fn message_delete_bulk(
        &self,
        _ctx: Context,
        channel_id: ChannelId,
        multiple_deleted_messages_ids: Vec<MessageId>,
        _guild_id: Option<serenity::all::GuildId>,
    ) {
        let channel_id_raw = channel_id.get();
        if !self.config.allows_channel(channel_id_raw) {
            return;
        }
        let gateway = GatewayId(self.gateway_id.clone());
        let channel = discord_channel_key(
            &gateway,
            channel_id_raw,
            _guild_id.map(|guild_id| guild_id.get()),
            None,
        );
        let deleted_at = chrono::Utc::now().timestamp();

        for message_id in multiple_deleted_messages_ids {
            self.runtime
                .emit_message_deleted(
                    &self.gateway_id,
                    channel.clone(),
                    message_id.to_string(),
                    deleted_at,
                )
                .await;
        }
    }
}

fn discord_channel_key(
    gateway_id: &GatewayId,
    channel_id: u64,
    guild_id: Option<u64>,
    parent_channel_id: Option<u64>,
) -> ProtocolChannelKey {
    let space = guild_id.map(|guild_id| SpaceKey {
        gateway_id: gateway_id.clone(),
        external_id: guild_id.to_string(),
        kind: SpaceKind::DiscordGuild,
        display_name: None,
        metadata: serde_json::json!({
            "platform": "discord",
        }),
    });
    ProtocolChannelKey {
        gateway_id: gateway_id.clone(),
        external_id: channel_id.to_string(),
        kind: if guild_id.is_some() {
            ChannelKind::PublicChannel
        } else {
            ChannelKind::Direct
        },
        display_name: None,
        space: space.clone(),
        parent: parent_channel_id.map(|parent| ParentChannelKey {
            gateway_id: gateway_id.clone(),
            external_id: parent.to_string(),
            kind: ChannelKind::PublicChannel,
            display_name: None,
            space,
            metadata: serde_json::json!({
                "platform": "discord",
            }),
        }),
        metadata: serde_json::json!({
            "platform": "discord",
        }),
    }
}

fn discord_identity_key(gateway_id: &GatewayId, user_id: u64, source: &str) -> ObservedIdentityKey {
    ObservedIdentityKey {
        gateway_id: gateway_id.clone(),
        external_id: user_id.to_string(),
        kind: Some("discord_user".into()),
        confidence: 1.0,
        source: source.to_string(),
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
