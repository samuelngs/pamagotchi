use media::{MediaStore, NewMediaAsset};
use protocol::{MediaAssetId, MediaAttachment, MediaKind};
use tracing::warn;
use whatsapp_rust::Client;
use whatsapp_rust::download::Downloadable;
use whatsapp_rust::waproto::whatsapp as wa;

pub(super) async fn extract_message_content(
    client: &Client,
    media_store: &MediaStore,
    msg: &wa::Message,
) -> (String, Vec<MediaAttachment>) {
    if let Some(ref text) = msg.conversation {
        return (text.clone(), Vec::new());
    }

    if let Some(ref ext) = msg.extended_text_message {
        if let Some(ref text) = ext.text {
            return (text.clone(), Vec::new());
        }
    }

    if let Some(ref img) = msg.image_message {
        return (
            img.caption.clone().unwrap_or_default(),
            vec![
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
            ],
        );
    }

    if let Some(ref vid) = msg.video_message {
        return (
            vid.caption.clone().unwrap_or_default(),
            vec![
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
            ],
        );
    }

    if let Some(ref aud) = msg.audio_message {
        return (
            String::new(),
            vec![
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
            ],
        );
    }

    if let Some(ref stk) = msg.sticker_message {
        return (
            String::new(),
            vec![
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
            ],
        );
    }

    if let Some(ref doc) = msg.document_message {
        return (
            doc.caption.clone().unwrap_or_default(),
            vec![
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
            ],
        );
    }

    (String::new(), Vec::new())
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

pub(super) fn build_media_attachment(
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
