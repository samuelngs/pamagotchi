use super::{MediaAttachment, MediaKind, PlatformAdapter, PlatformCapabilities};
use crate::core::event::{InboundMessage, WakeEvent};
use crate::identity::GroupId;
use crate::store::ConversationId;
use async_trait::async_trait;
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
    client: Arc<Client>,
}

impl WhatsAppAdapter {
    pub async fn connect(
        db_path: &str,
        event_tx: mpsc::Sender<WakeEvent>,
    ) -> anyhow::Result<Self> {
        let backend = SqliteStore::new(db_path).await?;

        let tx = event_tx.clone();
        let mut bot = Bot::builder()
            .with_backend(Arc::new(backend))
            .with_transport_factory(TokioWebSocketTransportFactory::new())
            .with_http_client(UreqHttpClient::new())
            .with_runtime(TokioRuntime)
            .on_event(move |event: Arc<Event>, _client: Arc<Client>| {
                let tx = tx.clone();
                async move {
                    handle_event(&event, &tx).await;
                }
            })
            .build()
            .await?;

        let client = bot.client();

        tokio::spawn(async move {
            match bot.run().await {
                Ok(handle) => {
                    if let Err(e) = handle.await {
                        error!("whatsapp disconnected: {e}");
                    }
                }
                Err(e) => error!("whatsapp failed to start: {e}"),
            }
        });

        info!("whatsapp adapter connected");

        Ok(Self { client })
    }
}

fn render_qr_compact(qr: &qrcode::QrCode) -> String {
    use qrcode::Color;
    let w = qr.width() as usize;
    let margin = 2;
    let total = w + margin * 2;
    let mut out = String::new();
    for row in (0..total).step_by(2) {
        for col in 0..total {
            let top = if row >= margin && row < margin + w && col >= margin && col < margin + w {
                qr[(row - margin, col - margin)] == Color::Dark
            } else {
                false
            };
            let bot = if row + 1 >= margin && row + 1 < margin + w && col >= margin && col < margin + w {
                qr[(row + 1 - margin, col - margin)] == Color::Dark
            } else {
                false
            };
            out.push(match (top, bot) {
                (true, true) => '\u{2588}',
                (true, false) => '\u{2580}',
                (false, true) => '\u{2584}',
                (false, false) => ' ',
            });
        }
        out.push('\n');
    }
    out
}

async fn handle_event(event: &Event, tx: &mpsc::Sender<WakeEvent>) {
    match event {
        Event::PairingQrCode { code, .. } => {
            info!("whatsapp pairing QR code received");
            if let Ok(qr) = qrcode::QrCode::new(code.as_bytes()) {
                eprintln!("\n{}\n", render_qr_compact(&qr));
            } else {
                warn!("failed to generate QR code, raw: {code}");
            }
        }
        Event::Connected(_) => {
            info!("whatsapp connected");
        }
        Event::Disconnected(_) => {
            warn!("whatsapp disconnected");
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
                platform_id: "whatsapp".into(),
                external_id: chat.clone(),
                conversation: ConversationId(format!("whatsapp:{chat}")),
                group: if info.source.is_group {
                    Some(GroupId(chat))
                } else {
                    None
                },
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

            if let Err(e) = tx.send(WakeEvent::Message(inbound)).await {
                warn!("failed to forward whatsapp message: {e}");
            }
        }
        _ => {}
    }
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
impl PlatformAdapter for WhatsAppAdapter {
    fn platform_id(&self) -> &str {
        "whatsapp"
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            content_types: vec![
                MediaKind::Image,
                MediaKind::Video,
                MediaKind::Audio,
                MediaKind::Sticker,
                MediaKind::File,
            ],
            composing: true,
            read_receipts: true,
        }
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
            // TODO: implement media upload + send
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
