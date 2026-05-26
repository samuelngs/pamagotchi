use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayContentKind, GatewayRuntime, GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
use media::MediaStore;
use protocol::{ConversationId, GroupId, InboundMessage};
use protocol::{MediaAttachment, MediaKind};
use qrcode::{QrCode, render::unicode};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use whatsapp_rust::bot::Bot;
use whatsapp_rust::proto_helpers::MessageExt;
use whatsapp_rust::types::events::Event;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{Client, Jid, TokioRuntime};
use whatsapp_rust_sqlite_storage::SqliteStore;
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

pub struct WhatsAppAdapter {
    id: String,
    client: Arc<Client>,
    runtime: Arc<GatewayRuntime>,
    _media_store: Arc<MediaStore>,
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
        let mut bot = Bot::builder()
            .with_backend(Arc::new(backend))
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .with_runtime(TokioRuntime)
            .on_event(move |event: Arc<Event>, _client: Arc<Client>| {
                let tx = tx.clone();
                let gateway_id = gateway_id.clone();
                let runtime = runtime_for_events.clone();
                async move {
                    handle_event(&gateway_id, &event, &tx, &runtime).await;
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
            _media_store: media_store,
        })
    }
}

async fn handle_event(
    gateway_id: &str,
    event: &Event,
    tx: &mpsc::Sender<InboundMessage>,
    runtime: &GatewayRuntime,
) {
    match event {
        Event::PairingQrCode { code, .. } => {
            info!("whatsapp pairing QR code received");
            let rendered = render_qr_compact(code);
            let setup = Some(GatewaySetupInstructions::QrCode {
                title: "Connect WhatsApp".into(),
                body: "Scan this QR code from WhatsApp > Linked devices.".into(),
                code: code.clone(),
                rendered,
            });
            runtime
                .emit_state(gateway_id, GatewayConnectionState::SetupRequired)
                .await;
            runtime.emit_setup(gateway_id, setup).await;
        }
        Event::Connected(_) => {
            info!("whatsapp connected");
            runtime
                .emit_state(gateway_id, GatewayConnectionState::Connected)
                .await;
            runtime.emit_setup(gateway_id, None).await;
        }
        Event::Disconnected(_) => {
            warn!("whatsapp disconnected");
            runtime
                .emit_state(gateway_id, GatewayConnectionState::Disconnected)
                .await;
        }
        Event::Message(msg, info) => {
            if info.source.is_from_me {
                debug!(message_id = %info.id, "dropping self-message (is_from_me)");
                return;
            }

            let base = msg.get_base_message();
            let (content, media) = extract_message_content(base);

            if content.is_empty() && media.is_none() {
                return;
            }

            let sender = info.source.sender.to_string();
            let chat = info.source.chat.to_string();

            let inbound = InboundMessage {
                message_id: info.id.to_string(),
                gateway_id: gateway_id.to_string(),
                external_id: chat.clone(),
                conversation: ConversationId(format!("{gateway_id}:{chat}")),
                group: if info.source.is_group {
                    Some(GroupId(chat))
                } else {
                    None
                },
                identity: None,
                profile: None,
                person: None,
                content,
                media,
                timestamp: info.timestamp.timestamp(),
                metadata: serde_json::json!({
                    "sender": sender,
                    "message_id": info.id.to_string(),
                    "push_name": info.push_name,
                }),
            };

            if let Err(e) = tx.send(inbound).await {
                warn!("failed to forward whatsapp message: {e}");
            }
        }
        _ => {}
    }
}

fn render_qr_compact(code: &str) -> String {
    QrCode::new(code.as_bytes())
        .map(|qr| qr.render::<unicode::Dense1x2>().quiet_zone(false).build())
        .unwrap_or_default()
}

fn extract_message_content(msg: &wa::Message) -> (String, Option<MediaAttachment>) {
    if let Some(ref text) = msg.conversation {
        return (text.clone(), None);
    }

    if let Some(ref ext) = msg.extended_text_message {
        if let Some(ref text) = ext.text {
            return (text.clone(), None);
        }
    }

    if let Some(ref img) = msg.image_message {
        return (
            img.caption.clone().unwrap_or_default(),
            Some(MediaAttachment {
                kind: MediaKind::Image,
                asset_id: None,
                url: img.direct_path.clone(),
                mime: img.mimetype.clone(),
                filename: None,
                size: img.file_length,
            }),
        );
    }

    if let Some(ref vid) = msg.video_message {
        return (
            vid.caption.clone().unwrap_or_default(),
            Some(MediaAttachment {
                kind: MediaKind::Video,
                asset_id: None,
                url: vid.direct_path.clone(),
                mime: vid.mimetype.clone(),
                filename: None,
                size: vid.file_length,
            }),
        );
    }

    if let Some(ref aud) = msg.audio_message {
        return (
            String::new(),
            Some(MediaAttachment {
                kind: MediaKind::Audio,
                asset_id: None,
                url: aud.direct_path.clone(),
                mime: aud.mimetype.clone(),
                filename: None,
                size: aud.file_length,
            }),
        );
    }

    if let Some(ref stk) = msg.sticker_message {
        return (
            String::new(),
            Some(MediaAttachment {
                kind: MediaKind::Sticker,
                asset_id: None,
                url: stk.direct_path.clone(),
                mime: stk.mimetype.clone(),
                filename: None,
                size: stk.file_length,
            }),
        );
    }

    if let Some(ref doc) = msg.document_message {
        return (
            doc.caption.clone().unwrap_or_default(),
            Some(MediaAttachment {
                kind: MediaKind::File,
                asset_id: None,
                url: doc.direct_path.clone(),
                mime: doc.mimetype.clone(),
                filename: doc.file_name.clone(),
                size: doc.file_length,
            }),
        );
    }

    (String::new(), None)
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
                send: vec![GatewayContentKind::Text],
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
        media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()> {
        let jid: Jid = external_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp JID: {external_id}"))?;

        let message = if let Some(_media) = media {
            warn!("media sending not yet implemented, sending text only");
            wa::Message {
                conversation: Some(content.to_string()),
                ..Default::default()
            }
        } else {
            wa::Message {
                conversation: Some(content.to_string()),
                ..Default::default()
            }
        };

        self.client.send_message(jid, message).await?;
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
