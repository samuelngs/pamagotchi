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
use protocol::{InboundMessage, MediaAttachment};
use serde_json::Value;
use serenity::all::{Client, GatewayIntents, Http};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct DiscordAdapter {
    id: String,
    http: Option<Arc<Http>>,
    runtime: Arc<GatewayRuntime>,
    _media_store: Arc<MediaStore>,
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
            _media_store: media_store,
        }
    }

    pub async fn connect_with_config(
        id: impl Into<String>,
        config: DiscordConfig,
        inbound_tx: mpsc::Sender<InboundMessage>,
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
            | GatewayIntents::MESSAGE_CONTENT;
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
            _media_store: media_store,
        })
    }
}

#[async_trait]
impl GatewayAdapter for DiscordAdapter {
    async fn connect(
        id: String,
        _db_path: String,
        vars: BTreeMap<String, Value>,
        inbound_tx: mpsc::Sender<InboundMessage>,
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
                send: vec![GatewayContentKind::Text],
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
        if !attachments.is_empty() {
            warn!("discord media sending not yet implemented, sending text only");
        }

        let http = self
            .http
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Discord gateway is not configured"))?;
        let channel_id = parse_channel_id(external_id)?;
        channel_id.say(http, content).await?;
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
