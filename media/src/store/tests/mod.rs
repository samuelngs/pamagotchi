use serde_json::json;

use super::*;

#[test]
fn put_bytes_writes_content_addressed_object_and_index() {
    let root = temp_root();
    let store = MediaStore::open(&root).unwrap();
    let bytes = b"image bytes";

    let asset = store
        .put_bytes(
            bytes,
            NewMediaAsset::new(MediaKind::Image)
                .with_mime("image/png")
                .with_filename("avatar.png")
                .with_metadata(json!({ "source": "whatsapp" })),
        )
        .unwrap();

    assert_eq!(asset.kind, MediaKind::Image);
    assert_eq!(asset.mime.as_deref(), Some("image/png"));
    assert_eq!(asset.filename.as_deref(), Some("avatar.png"));
    assert_eq!(asset.size, bytes.len() as u64);
    assert_eq!(asset.sha256.len(), 64);
    assert!(asset.path.exists());
    assert!(asset.path.starts_with(root.join(OBJECTS_DIR)));

    let loaded = store.get(&asset.id).unwrap().unwrap();
    assert_eq!(loaded, asset);
    assert_eq!(loaded.metadata, json!({ "source": "whatsapp" }));
    assert_eq!(store.read_bytes(&asset.id).unwrap().unwrap(), bytes);
}

#[test]
fn same_payload_reuses_object_path_but_keeps_distinct_asset_rows() {
    let root = temp_root();
    let store = MediaStore::open(&root).unwrap();

    let first = store
        .put_bytes(b"same bytes", NewMediaAsset::new(MediaKind::Sticker))
        .unwrap();
    let second = store
        .put_bytes(b"same bytes", NewMediaAsset::new(MediaKind::File))
        .unwrap();

    assert_ne!(first.id, second.id);
    assert_eq!(first.path, second.path);
    assert_eq!(first.sha256, second.sha256);
}

#[test]
fn missing_asset_returns_none() {
    let store = MediaStore::open(temp_root()).unwrap();
    let id = MediaAssetId("media-missing".to_string());

    assert!(store.get(&id).unwrap().is_none());
    assert!(store.path_for(&id).unwrap().is_none());
    assert!(store.read_bytes(&id).unwrap().is_none());
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!("pamagotchi-media-{}", nanoid::nanoid!(12)))
}
