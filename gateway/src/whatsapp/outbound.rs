use protocol::{MediaAttachment, MediaKind};
use whatsapp_rust::download::MediaType;
use whatsapp_rust::upload::UploadResponse;
use whatsapp_rust::waproto::whatsapp as wa;

pub(super) fn whatsapp_media_type(kind: &MediaKind) -> MediaType {
    match kind {
        MediaKind::Image => MediaType::Image,
        MediaKind::Video => MediaType::Video,
        MediaKind::Audio => MediaType::Audio,
        MediaKind::Sticker => MediaType::Sticker,
        MediaKind::File => MediaType::Document,
    }
}

pub(super) fn build_outbound_media_message(
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

pub(super) fn text_message(content: &str) -> wa::Message {
    wa::Message {
        conversation: Some(content.to_string()),
        ..Default::default()
    }
}
