use super::*;
use crate::GatewayAdapter;
use media::MediaStore;
use protocol::MediaKind;
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn parses_config_from_direct_token() {
    let vars = BTreeMap::from([
        ("bot_token".into(), Value::String("token".into())),
        (
            "allowed_channel_ids".into(),
            serde_json::json!(["123", "456"]),
        ),
        ("ignore_bots".into(), Value::Bool(false)),
    ]);

    let config = DiscordConfig::from_vars(&vars).unwrap();

    assert_eq!(config.bot_token, "token");
    assert!(config.allowed_channel_ids.contains(&123));
    assert!(config.allowed_channel_ids.contains(&456));
    assert!(!config.ignore_bots);
}

#[test]
fn defaults_optional_vars_when_only_token_is_set() {
    let vars = BTreeMap::from([("bot_token".into(), Value::String("token".into()))]);

    let config = DiscordConfig::from_vars(&vars).unwrap();

    assert_eq!(config.bot_token, "token");
    assert!(config.allowed_channel_ids.is_empty());
    assert!(config.ignore_bots);
}

#[test]
fn rejects_invalid_channel_ids() {
    let vars = BTreeMap::from([
        ("bot_token".into(), Value::String("token".into())),
        ("allowed_channel_ids".into(), serde_json::json!(["abc"])),
    ]);

    assert!(DiscordConfig::from_vars(&vars).is_err());
}

#[test]
fn requires_bot_token_in_vars() {
    let vars = BTreeMap::new();

    assert!(DiscordConfig::from_vars(&vars).is_err());
}

#[tokio::test]
async fn connect_without_bot_token_returns_setup_required_adapter() {
    let (inbound_tx, _inbound_rx) = mpsc::channel(1);
    let (event_tx, mut event_rx) = mpsc::channel(4);

    let adapter = DiscordAdapter::connect(
        "discord-1".into(),
        String::new(),
        BTreeMap::new(),
        inbound_tx,
        event_tx,
        Arc::new(MediaStore::open(temp_media_root()).unwrap()),
    )
    .await
    .unwrap();

    assert_eq!(
        adapter.connection_state(),
        GatewayConnectionState::SetupRequired
    );
    assert!(adapter.setup_instructions().is_some());
    assert!(adapter.http.is_none());
    assert!(adapter.send_message("123", "hello", &[]).await.is_err());

    assert!(matches!(
        event_rx.recv().await,
        Some(GatewayRuntimeEvent::ConnectionStateChanged {
            state: GatewayConnectionState::SetupRequired,
            ..
        })
    ));
    assert!(matches!(
        event_rx.recv().await,
        Some(GatewayRuntimeEvent::SetupInstructionsChanged { setup: Some(_), .. })
    ));
}

#[test]
fn maps_mime_to_media_kind() {
    assert!(matches!(
        message::media_kind_from_mime(Some("image/png")),
        MediaKind::Image
    ));
    assert!(matches!(
        message::media_kind_from_mime(Some("video/mp4")),
        MediaKind::Video
    ));
    assert!(matches!(
        message::media_kind_from_mime(Some("audio/mpeg")),
        MediaKind::Audio
    ));
    assert!(matches!(
        message::media_kind_from_mime(None),
        MediaKind::File
    ));
}

#[test]
fn maps_discord_attachment_fields_without_dropping_metadata() {
    let attachment = message::discord_attachment(
        "https://cdn.example.test/image.png".into(),
        Some("image/png".into()),
        "image.png".into(),
        42,
    );

    assert_eq!(attachment.kind, MediaKind::Image);
    assert_eq!(
        attachment.url.as_deref(),
        Some("https://cdn.example.test/image.png")
    );
    assert_eq!(attachment.mime.as_deref(), Some("image/png"));
    assert_eq!(attachment.filename.as_deref(), Some("image.png"));
    assert_eq!(attachment.size, Some(42));
}

fn temp_media_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pamagotchi-gateway-media-{}", nanoid::nanoid!(12)))
}
