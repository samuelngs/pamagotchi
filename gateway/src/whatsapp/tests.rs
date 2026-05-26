use super::*;
use protocol::MediaAssetId;
use whatsapp_rust::download::MediaType;
use whatsapp_rust::upload::UploadResponse;

#[test]
fn build_media_attachment_preserves_asset_and_direct_path() {
    let attachment = inbound::build_media_attachment(
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
    assert_eq!(
        outbound::whatsapp_media_type(&MediaKind::Image),
        MediaType::Image
    );
    assert_eq!(
        outbound::whatsapp_media_type(&MediaKind::Video),
        MediaType::Video
    );
    assert_eq!(
        outbound::whatsapp_media_type(&MediaKind::Audio),
        MediaType::Audio
    );
    assert_eq!(
        outbound::whatsapp_media_type(&MediaKind::Sticker),
        MediaType::Sticker
    );
    assert_eq!(
        outbound::whatsapp_media_type(&MediaKind::File),
        MediaType::Document
    );
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

    let message = outbound::build_outbound_media_message(&upload, &media, "caption");
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

    let message = outbound::build_outbound_media_message(&upload, &media, "caption");
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
