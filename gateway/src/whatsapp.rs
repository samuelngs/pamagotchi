use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayContentKind, GatewayRuntime, GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
use media::{MediaStore, NewMediaAsset};
use protocol::{ConversationId, GroupId, InboundMessage, MediaAssetId};
use protocol::{MediaAttachment, MediaKind};
use qrcode::{QrCode, render::unicode};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use whatsapp_rust::bot::Bot;
use whatsapp_rust::download::{Downloadable, MediaType};
use whatsapp_rust::proto_helpers::MessageExt;
use whatsapp_rust::types::events::Event;
use whatsapp_rust::upload::UploadResponse;
use whatsapp_rust::waproto::whatsapp as wa;
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
            .on_event(move |event: Arc<Event>, client: Arc<Client>| {
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

async fn handle_event(
    gateway_id: &str,
    event: &Event,
    tx: &mpsc::Sender<InboundMessage>,
    runtime: &GatewayRuntime,
    client: &Client,
    media_store: &MediaStore,
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
            let (content, media) = extract_message_content(client, media_store, base).await;

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

async fn extract_message_content(
    client: &Client,
    media_store: &MediaStore,
    msg: &wa::Message,
) -> (String, Option<MediaAttachment>) {
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
            Some(
                persist_media_attachment(
                    client,
                    media_store,
                    img.as_ref(),
                    MediaKind::Image,
                    "image",
                    img.mimetype.clone(),
                    None,
                    img.file_length,
                )
                .await,
            ),
        );
    }

    if let Some(ref vid) = msg.video_message {
        return (
            vid.caption.clone().unwrap_or_default(),
            Some(
                persist_media_attachment(
                    client,
                    media_store,
                    vid.as_ref(),
                    MediaKind::Video,
                    "video",
                    vid.mimetype.clone(),
                    None,
                    vid.file_length,
                )
                .await,
            ),
        );
    }

    if let Some(ref aud) = msg.audio_message {
        return (
            String::new(),
            Some(
                persist_media_attachment(
                    client,
                    media_store,
                    aud.as_ref(),
                    MediaKind::Audio,
                    "audio",
                    aud.mimetype.clone(),
                    None,
                    aud.file_length,
                )
                .await,
            ),
        );
    }

    if let Some(ref stk) = msg.sticker_message {
        return (
            String::new(),
            Some(
                persist_media_attachment(
                    client,
                    media_store,
                    stk.as_ref(),
                    MediaKind::Sticker,
                    "sticker",
                    stk.mimetype.clone(),
                    None,
                    stk.file_length,
                )
                .await,
            ),
        );
    }

    if let Some(ref doc) = msg.document_message {
        return (
            doc.caption.clone().unwrap_or_default(),
            Some(
                persist_media_attachment(
                    client,
                    media_store,
                    doc.as_ref(),
                    MediaKind::File,
                    "document",
                    doc.mimetype.clone(),
                    doc.file_name.clone(),
                    doc.file_length,
                )
                .await,
            ),
        );
    }

    (String::new(), None)
}

async fn persist_media_attachment(
    client: &Client,
    media_store: &MediaStore,
    downloadable: &dyn Downloadable,
    kind: MediaKind,
    whatsapp_media_type: &'static str,
    mime: Option<String>,
    filename: Option<String>,
    size: Option<u64>,
) -> MediaAttachment {
    let direct_path = downloadable.direct_path().map(ToString::to_string);
    let fallback = build_media_attachment(
        kind.clone(),
        None,
        direct_path.clone(),
        mime.clone(),
        filename.clone(),
        size,
    );

    let bytes = match client.download(downloadable).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(
                %e,
                kind = kind.as_str(),
                direct_path = direct_path.as_deref().unwrap_or_default(),
                "failed to download whatsapp inbound media"
            );
            return fallback;
        }
    };

    let new_asset = NewMediaAsset {
        kind: kind.clone(),
        mime: mime.clone(),
        filename: filename.clone(),
        metadata: serde_json::json!({
            "platform": "whatsapp",
            "media_type": whatsapp_media_type,
            "direct_path": direct_path.clone(),
            "declared_size": size,
        }),
    };

    match media_store.put_bytes(&bytes, new_asset) {
        Ok(asset) => build_media_attachment(
            kind,
            Some(asset.id),
            direct_path,
            mime,
            filename,
            Some(asset.size),
        ),
        Err(e) => {
            warn!(
                %e,
                kind = kind.as_str(),
                "failed to store whatsapp inbound media"
            );
            fallback
        }
    }
}

fn build_media_attachment(
    kind: MediaKind,
    asset_id: Option<MediaAssetId>,
    url: Option<String>,
    mime: Option<String>,
    filename: Option<String>,
    size: Option<u64>,
) -> MediaAttachment {
    MediaAttachment {
        kind,
        asset_id,
        url,
        mime,
        filename,
        size,
    }
}

fn whatsapp_media_type(kind: &MediaKind) -> MediaType {
    match kind {
        MediaKind::Image => MediaType::Image,
        MediaKind::Video => MediaType::Video,
        MediaKind::Audio => MediaType::Audio,
        MediaKind::Sticker => MediaType::Sticker,
        MediaKind::File => MediaType::Document,
    }
}

fn build_outbound_media_message(
    upload: &UploadResponse,
    media: &MediaAttachment,
    content: &str,
) -> wa::Message {
    let caption = (!content.is_empty()).then(|| content.to_string());
    match media.kind {
        MediaKind::Image => wa::Message {
            image_message: Some(Box::new(wa::message::ImageMessage {
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key_vec()),
                file_sha256: Some(upload.file_sha256_vec()),
                file_enc_sha256: Some(upload.file_enc_sha256_vec()),
                file_length: Some(upload.file_length),
                mimetype: media.mime.clone(),
                caption,
                ..Default::default()
            })),
            ..Default::default()
        },
        MediaKind::Video => wa::Message {
            video_message: Some(Box::new(wa::message::VideoMessage {
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key_vec()),
                file_sha256: Some(upload.file_sha256_vec()),
                file_enc_sha256: Some(upload.file_enc_sha256_vec()),
                file_length: Some(upload.file_length),
                mimetype: media.mime.clone(),
                caption,
                ..Default::default()
            })),
            ..Default::default()
        },
        MediaKind::Audio => wa::Message {
            audio_message: Some(Box::new(wa::message::AudioMessage {
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key_vec()),
                file_sha256: Some(upload.file_sha256_vec()),
                file_enc_sha256: Some(upload.file_enc_sha256_vec()),
                file_length: Some(upload.file_length),
                mimetype: media.mime.clone(),
                ptt: Some(false),
                ..Default::default()
            })),
            ..Default::default()
        },
        MediaKind::Sticker => wa::Message {
            sticker_message: Some(Box::new(wa::message::StickerMessage {
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key_vec()),
                file_sha256: Some(upload.file_sha256_vec()),
                file_enc_sha256: Some(upload.file_enc_sha256_vec()),
                file_length: Some(upload.file_length),
                mimetype: media
                    .mime
                    .clone()
                    .or_else(|| Some("image/webp".to_string())),
                ..Default::default()
            })),
            ..Default::default()
        },
        MediaKind::File => wa::Message {
            document_message: Some(Box::new(wa::message::DocumentMessage {
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key_vec()),
                file_sha256: Some(upload.file_sha256_vec()),
                file_enc_sha256: Some(upload.file_enc_sha256_vec()),
                file_length: Some(upload.file_length),
                mimetype: media.mime.clone(),
                file_name: media.filename.clone(),
                caption,
                ..Default::default()
            })),
            ..Default::default()
        },
    }
}

fn text_message(content: &str) -> wa::Message {
    wa::Message {
        conversation: Some(content.to_string()),
        ..Default::default()
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
        media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()> {
        let jid: Jid = external_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp JID: {external_id}"))?;

        let Some(media) = media else {
            self.client.send_message(jid, text_message(content)).await?;
            return Ok(());
        };

        let asset_id = media
            .asset_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("WhatsApp media send requires a stored media asset"))?;
        let bytes = self
            .media_store
            .read_bytes(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("media asset not found: {}", asset_id.0))?;
        let upload = self
            .client
            .upload(bytes, whatsapp_media_type(&media.kind), Default::default())
            .await?;

        if !content.is_empty() && matches!(media.kind, MediaKind::Audio | MediaKind::Sticker) {
            self.client
                .send_message(jid.clone(), text_message(content))
                .await?;
        }

        let message = build_outbound_media_message(&upload, media, content);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_media_attachment_preserves_asset_and_direct_path() {
        let attachment = build_media_attachment(
            MediaKind::Sticker,
            Some(MediaAssetId("media-test".into())),
            Some("/mms/image/path".into()),
            Some("image/webp".into()),
            Some("sticker.webp".into()),
            Some(42),
        );

        assert_eq!(attachment.kind, MediaKind::Sticker);
        assert_eq!(attachment.asset_id, Some(MediaAssetId("media-test".into())));
        assert_eq!(attachment.url.as_deref(), Some("/mms/image/path"));
        assert_eq!(attachment.mime.as_deref(), Some("image/webp"));
        assert_eq!(attachment.filename.as_deref(), Some("sticker.webp"));
        assert_eq!(attachment.size, Some(42));
    }

    #[test]
    fn maps_media_kind_to_whatsapp_media_type() {
        assert_eq!(whatsapp_media_type(&MediaKind::Image), MediaType::Image);
        assert_eq!(whatsapp_media_type(&MediaKind::Video), MediaType::Video);
        assert_eq!(whatsapp_media_type(&MediaKind::Audio), MediaType::Audio);
        assert_eq!(whatsapp_media_type(&MediaKind::Sticker), MediaType::Sticker);
        assert_eq!(whatsapp_media_type(&MediaKind::File), MediaType::Document);
    }

    #[test]
    fn builds_image_message_from_upload_response() {
        let upload = upload_response();
        let media = MediaAttachment {
            kind: MediaKind::Image,
            asset_id: Some(MediaAssetId("media-test".into())),
            url: None,
            mime: Some("image/png".into()),
            filename: None,
            size: Some(upload.file_length),
        };

        let message = build_outbound_media_message(&upload, &media, "caption");
        let image = message.image_message.unwrap();

        assert_eq!(image.url.as_deref(), Some("https://cdn.example/upload"));
        assert_eq!(image.direct_path.as_deref(), Some("/mms/image/path"));
        assert_eq!(image.media_key, Some(vec![1; 32]));
        assert_eq!(image.file_sha256, Some(vec![2; 32]));
        assert_eq!(image.file_enc_sha256, Some(vec![3; 32]));
        assert_eq!(image.file_length, Some(99));
        assert_eq!(image.mimetype.as_deref(), Some("image/png"));
        assert_eq!(image.caption.as_deref(), Some("caption"));
    }

    #[test]
    fn builds_document_message_from_upload_response() {
        let upload = upload_response();
        let media = MediaAttachment {
            kind: MediaKind::File,
            asset_id: Some(MediaAssetId("media-test".into())),
            url: None,
            mime: Some("application/pdf".into()),
            filename: Some("report.pdf".into()),
            size: Some(upload.file_length),
        };

        let message = build_outbound_media_message(&upload, &media, "caption");
        let document = message.document_message.unwrap();

        assert_eq!(document.file_name.as_deref(), Some("report.pdf"));
        assert_eq!(document.mimetype.as_deref(), Some("application/pdf"));
        assert_eq!(document.caption.as_deref(), Some("caption"));
        assert_eq!(document.media_key, Some(vec![1; 32]));
    }

    fn upload_response() -> UploadResponse {
        UploadResponse {
            url: "https://cdn.example/upload".into(),
            direct_path: "/mms/image/path".into(),
            media_key: [1; 32],
            file_sha256: [2; 32],
            file_enc_sha256: [3; 32],
            file_length: 99,
            media_key_timestamp: 123,
        }
    }
}
