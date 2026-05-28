use super::*;

#[test]
fn media_attachment_defaults_missing_asset_id() {
    let attachment: MediaAttachment = serde_json::from_value(serde_json::json!({
        "kind": "Image",
        "url": "https://example.test/image.png",
        "mime": "image/png",
        "filename": "image.png",
        "size": 10
    }))
    .unwrap();

    assert_eq!(attachment.kind, MediaKind::Image);
    assert_eq!(attachment.asset_id, None);
}

#[test]
fn media_kind_parse_accepts_wire_and_human_names() {
    assert_eq!(MediaKind::parse("image"), Some(MediaKind::Image));
    assert_eq!(MediaKind::parse("Image"), Some(MediaKind::Image));
    assert_eq!(MediaKind::parse("voice"), Some(MediaKind::Audio));
    assert_eq!(MediaKind::parse("Voice"), Some(MediaKind::Audio));
    assert_eq!(MediaKind::parse("file"), Some(MediaKind::File));
    assert_eq!(MediaKind::parse("File"), Some(MediaKind::File));
}
