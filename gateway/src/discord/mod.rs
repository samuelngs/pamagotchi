mod config;
mod message;

#[cfg(test)]
mod tests;

use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayContentKind, GatewayRuntime, GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
pub use config::DiscordConfig;
use config::{is_missing_bot_token_error, setup_instructions};
use media::MediaStore;
use message::{DiscordHandler, parse_channel_id};
use protocol::{InboundEnvelope, MediaAttachment};
use serde_json::Value;
use serenity::all::{Client, GatewayIntents, Http};
use serenity::builder::{CreateAttachment, CreateMessage};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

pub struct DiscordAdapter {
    id: String,
    http: Option<Arc<Http>>,
    runtime: Arc<GatewayRuntime>,
    media_store: Arc<MediaStore>,
}

impl DiscordAdapter {
    async fn setup_required(
        id: impl Into<String>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> Self {
        let id = id.into();
        let runtime = Arc::new(GatewayRuntime::new(gateway_event_tx));
        runtime
            .emit_state(&id, GatewayConnectionState::SetupRequired)
            .await;
        runtime.emit_setup(&id, Some(setup_instructions())).await;

        Self {
            id,
            http: None,
            runtime,
            media_store,
        }
    }

    pub async fn connect_with_config(
        id: impl Into<String>,
        config: DiscordConfig,
        inbound_tx: mpsc::Sender<InboundEnvelope>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        let id = id.into();
        let runtime = Arc::new(GatewayRuntime::new(gateway_event_tx));
        runtime
            .emit_state(&id, GatewayConnectionState::Connecting)
            .await;
        runtime.emit_setup(&id, Some(setup_instructions())).await;

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_TYPING
            | GatewayIntents::DIRECT_MESSAGE_TYPING;
        let handler = DiscordHandler {
            gateway_id: id.clone(),
            config: config.clone(),
            inbound_tx,
            runtime: runtime.clone(),
        };

        let mut client = Client::builder(&config.bot_token, intents)
            .event_handler(handler)
            .await?;
        let http = client.http.clone();
        let runtime_for_run = runtime.clone();
        let gateway_id_for_run = id.clone();

        tokio::spawn(async move {
            if let Err(e) = client.start().await {
                error!(%e, gateway = %gateway_id_for_run, "discord gateway stopped with error");
                runtime_for_run
                    .emit_state(
                        &gateway_id_for_run,
                        GatewayConnectionState::Error {
                            message: e.to_string(),
                        },
                    )
                    .await;
            }
        });

        info!(gateway = %id, "discord adapter started");

        Ok(Self {
            id,
            http: Some(http),
            runtime,
            media_store,
        })
    }
}

#[async_trait]
impl GatewayAdapter for DiscordAdapter {
    async fn connect(
        id: String,
        _db_path: String,
        vars: BTreeMap<String, Value>,
        inbound_tx: mpsc::Sender<InboundEnvelope>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        let config = match DiscordConfig::from_vars(&vars) {
            Ok(config) => config,
            Err(e) if is_missing_bot_token_error(&e) => {
                return Ok(Self::setup_required(id, gateway_event_tx, media_store).await);
            }
            Err(e) => return Err(e),
        };

        Self::connect_with_config(id, config, inbound_tx, gateway_event_tx, media_store).await
    }

    fn kind(&self) -> &str {
        "discord"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities {
                receive: vec![
                    GatewayContentKind::Text,
                    GatewayContentKind::Image,
                    GatewayContentKind::Video,
                    GatewayContentKind::Audio,
                    GatewayContentKind::File,
                ],
                send: vec![
                    GatewayContentKind::Text,
                    GatewayContentKind::Image,
                    GatewayContentKind::Video,
                    GatewayContentKind::Audio,
                    GatewayContentKind::File,
                ],
            },
            composing: true,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        &self.id
    }

    fn connection_state(&self) -> GatewayConnectionState {
        self.runtime.connection_state()
    }

    fn setup_instructions(&self) -> Option<GatewaySetupInstructions> {
        self.runtime.setup_instructions()
    }

    async fn send_message(
        &self,
        external_id: &str,
        content: &str,
        attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Discord gateway is not configured"))?;
        let channel_id = parse_channel_id(external_id)?;
        if attachments.is_empty() {
            channel_id.say(http, content).await?;
        } else {
            let files = discord_attachments(http, &self.media_store, attachments).await?;
            let mut builder = CreateMessage::new();
            if !content.trim().is_empty() {
                builder = builder.content(content);
            }
            channel_id.send_files(http, files, builder).await?;
        }
        Ok(())
    }

    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Discord gateway is not configured"))?;
        let channel_id = parse_channel_id(external_id)?;
        channel_id.broadcast_typing(http).await?;
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

async fn discord_attachments(
    http: &Arc<Http>,
    media_store: &MediaStore,
    attachments: &[MediaAttachment],
) -> anyhow::Result<Vec<CreateAttachment>> {
    let mut files = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        files.push(discord_attachment(http, media_store, attachment).await?);
    }
    Ok(files)
}

async fn discord_attachment(
    http: &Arc<Http>,
    media_store: &MediaStore,
    attachment: &MediaAttachment,
) -> anyhow::Result<CreateAttachment> {
    if let Some(asset_id) = attachment.asset_id.as_ref() {
        let asset = media_store
            .get(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("media asset not found: {}", asset_id.0))?;
        let bytes = media_store
            .read_bytes(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("media asset not found: {}", asset_id.0))?;
        let filename = attachment
            .filename
            .as_deref()
            .or(asset.filename.as_deref())
            .map(str::to_string)
            .unwrap_or_else(|| default_attachment_filename(attachment));
        return Ok(CreateAttachment::bytes(bytes, filename));
    }

    if let Some(url) = attachment.url.as_deref() {
        return Ok(CreateAttachment::url(http, url).await?);
    }

    anyhow::bail!("Discord media send requires media_asset_id or media_url")
}

fn default_attachment_filename(attachment: &MediaAttachment) -> String {
    let extension = match attachment.mime.as_deref() {
        Some("image/jpeg") => "jpg",
        Some("image/png") => "png",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some("video/mp4") => "mp4",
        Some("audio/mpeg") => "mp3",
        Some("audio/ogg") => "ogg",
        Some("application/pdf") => "pdf",
        _ => match attachment.kind {
            protocol::MediaKind::Image => "png",
            protocol::MediaKind::Video => "mp4",
            protocol::MediaKind::Audio => "ogg",
            protocol::MediaKind::Sticker => "webp",
            protocol::MediaKind::File => "bin",
        },
    };
    format!("attachment.{extension}")
}
