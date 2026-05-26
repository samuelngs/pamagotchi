mod events;
mod inbound;
mod outbound;

#[cfg(test)]
mod tests;

use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayContentKind, GatewayRuntime, GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
use events::handle_event;
use media::MediaStore;
use outbound::{build_outbound_media_message, text_message, whatsapp_media_type};
use protocol::{InboundMessage, MediaAttachment, MediaKind};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};
use whatsapp_rust::bot::Bot;
use whatsapp_rust::{Client, Jid, TokioRuntime};
use whatsapp_rust_sqlite_storage::SqliteStore;
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

pub struct WhatsAppAdapter {
    id: String,
    client: Arc<Client>,
    runtime: Arc<GatewayRuntime>,
    media_store: Arc<MediaStore>,
}

impl WhatsAppAdapter {
    pub async fn connect_with_id(
        id: impl Into<String>,
        db_path: &str,
        inbound_tx: mpsc::Sender<InboundMessage>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        let id = id.into();
        let runtime = Arc::new(GatewayRuntime::new(gateway_event_tx));
        runtime
            .emit_state(&id, GatewayConnectionState::Connecting)
            .await;

        let backend = SqliteStore::new(db_path).await?;

        let tx = inbound_tx.clone();
        let gateway_id = id.clone();
        let runtime_for_events = runtime.clone();
        let media_store_for_events = media_store.clone();
        let mut bot = Bot::builder()
            .with_backend(Arc::new(backend))
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .with_runtime(TokioRuntime)
            .on_event(move |event, client| {
                let tx = tx.clone();
                let gateway_id = gateway_id.clone();
                let runtime = runtime_for_events.clone();
                let media_store = media_store_for_events.clone();
                async move {
                    handle_event(&gateway_id, &event, &tx, &runtime, &client, &media_store).await;
                }
            })
            .build()
            .await?;

        let client = bot.client();

        let runtime_for_run = runtime.clone();
        let gateway_id_for_run = id.clone();
        tokio::spawn(async move {
            match bot.run().await {
                Ok(handle) => {
                    if let Err(e) = handle.await {
                        error!("whatsapp disconnected: {e}");
                        runtime_for_run
                            .emit_state(
                                &gateway_id_for_run,
                                GatewayConnectionState::Error {
                                    message: e.to_string(),
                                },
                            )
                            .await;
                    }
                }
                Err(e) => {
                    error!("whatsapp failed to start: {e}");
                    runtime_for_run
                        .emit_state(
                            &gateway_id_for_run,
                            GatewayConnectionState::Error {
                                message: e.to_string(),
                            },
                        )
                        .await;
                }
            }
        });

        info!(gateway = %id, "whatsapp adapter connected");

        Ok(Self {
            id,
            client,
            runtime,
            media_store,
        })
    }
}

#[async_trait]
impl GatewayAdapter for WhatsAppAdapter {
    async fn connect(
        id: String,
        db_path: String,
        _vars: std::collections::BTreeMap<String, serde_json::Value>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        Self::connect_with_id(id, &db_path, inbound_tx, gateway_event_tx, media_store).await
    }

    fn gateway_id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "whatsapp"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities {
                receive: vec![
                    GatewayContentKind::Text,
                    GatewayContentKind::Image,
                    GatewayContentKind::Video,
                    GatewayContentKind::Audio,
                    GatewayContentKind::Sticker,
                    GatewayContentKind::File,
                ],
                send: vec![
                    GatewayContentKind::Text,
                    GatewayContentKind::Image,
                    GatewayContentKind::Video,
                    GatewayContentKind::Audio,
                    GatewayContentKind::Sticker,
                    GatewayContentKind::File,
                ],
            },
            composing: true,
            read_receipts: true,
        }
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
        let jid: Jid = external_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp JID: {external_id}"))?;

        if attachments.is_empty() {
            self.client.send_message(jid, text_message(content)).await?;
            return Ok(());
        }

        for (index, media) in attachments.iter().enumerate() {
            let attachment_content = if index == 0 { content } else { "" };
            let asset_id = media.asset_id.as_ref().ok_or_else(|| {
                anyhow::anyhow!("WhatsApp media send requires a stored media asset")
            })?;
            let bytes = self
                .media_store
                .read_bytes(asset_id)?
                .ok_or_else(|| anyhow::anyhow!("media asset not found: {}", asset_id.0))?;
            let upload = self
                .client
                .upload(bytes, whatsapp_media_type(&media.kind), Default::default())
                .await?;

            if !attachment_content.is_empty()
                && matches!(media.kind, MediaKind::Audio | MediaKind::Sticker)
            {
                self.client
                    .send_message(jid.clone(), text_message(attachment_content))
                    .await?;
            }

            let message = build_outbound_media_message(&upload, media, attachment_content);
            self.client.send_message(jid.clone(), message).await?;
        }
        Ok(())
    }

    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()> {
        let jid: Jid = external_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp JID: {external_id}"))?;
        self.client.chatstate().send_composing(&jid).await?;
        Ok(())
    }

    async fn stop_composing(&self, external_id: &str) -> anyhow::Result<()> {
        let jid: Jid = external_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp JID: {external_id}"))?;
        self.client.chatstate().send_paused(&jid).await?;
        Ok(())
    }
}
