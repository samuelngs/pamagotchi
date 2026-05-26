use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayContentKind, GatewayRuntime, GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
use media::MediaStore;
use protocol::{ConversationId, GroupId, InboundMessage, MediaAttachment, MediaKind};
use serde_json::Value;
use serenity::all::{
    ChannelId, Client, Context, EventHandler, GatewayIntents, Http, Message, Ready,
};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct DiscordConfig {
    pub bot_token: String,
    pub allowed_channel_ids: HashSet<u64>,
    pub ignore_bots: bool,
}

impl DiscordConfig {
    pub fn from_vars(vars: &BTreeMap<String, Value>) -> anyhow::Result<Self> {
        let bot_token = token_from_vars(vars)?;
        let allowed_channel_ids = string_array_var(vars, "allowed_channel_ids")?
            .into_iter()
            .map(|id| {
                id.parse::<u64>()
                    .map_err(|_| anyhow::anyhow!("invalid Discord channel id: {id}"))
            })
            .collect::<anyhow::Result<HashSet<_>>>()?;
        let ignore_bots = vars
            .get("ignore_bots")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        Ok(Self {
            bot_token,
            allowed_channel_ids,
            ignore_bots,
        })
    }

    fn allows_channel(&self, channel_id: u64) -> bool {
        self.allowed_channel_ids.is_empty() || self.allowed_channel_ids.contains(&channel_id)
    }
}

pub struct DiscordAdapter {
    id: String,
    http: Option<Arc<Http>>,
    runtime: Arc<GatewayRuntime>,
    _media_store: Arc<MediaStore>,
}

impl DiscordAdapter {
    async fn setup_required(
        id: impl Into<String>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> Self {
        let id = id.into();
        let runtime = Arc::new(GatewayRuntime::new(gateway_event_tx));
        runtime
            .emit_state(&id, GatewayConnectionState::SetupRequired)
            .await;
        runtime
            .emit_setup(&id, Some(discord_setup_instructions()))
            .await;

        Self {
            id,
            http: None,
            runtime,
            _media_store: media_store,
        }
    }

    pub async fn connect_with_config(
        id: impl Into<String>,
        config: DiscordConfig,
        inbound_tx: mpsc::Sender<InboundMessage>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        let id = id.into();
        let runtime = Arc::new(GatewayRuntime::new(gateway_event_tx));
        runtime
            .emit_state(&id, GatewayConnectionState::Connecting)
            .await;
        runtime
            .emit_setup(&id, Some(discord_setup_instructions()))
            .await;

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;
        let handler = DiscordHandler {
            gateway_id: id.clone(),
            config: config.clone(),
            inbound_tx,
            runtime: runtime.clone(),
        };

        let mut client = Client::builder(&config.bot_token, intents)
            .event_handler(handler)
            .await?;
        let http = client.http.clone();
        let runtime_for_run = runtime.clone();
        let gateway_id_for_run = id.clone();

        tokio::spawn(async move {
            if let Err(e) = client.start().await {
                error!(%e, gateway = %gateway_id_for_run, "discord gateway stopped with error");
                runtime_for_run
                    .emit_state(
                        &gateway_id_for_run,
                        GatewayConnectionState::Error {
                            message: e.to_string(),
                        },
                    )
                    .await;
            }
        });

        info!(gateway = %id, "discord adapter started");

        Ok(Self {
            id,
            http: Some(http),
            runtime,
            _media_store: media_store,
        })
    }
}

struct DiscordHandler {
    gateway_id: String,
    config: DiscordConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    runtime: Arc<GatewayRuntime>,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(
            gateway = %self.gateway_id,
            user = %ready.user.name,
            "discord gateway connected"
        );
        self.runtime
            .emit_state(&self.gateway_id, GatewayConnectionState::Connected)
            .await;
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        if self.config.ignore_bots && msg.author.bot {
            return;
        }

        let channel_id = msg.channel_id.get();
        if !self.config.allows_channel(channel_id) {
            return;
        }

        let (content, attachments) = extract_message_content(&msg);
        if content.trim().is_empty() && attachments.is_empty() {
            return;
        }

        let inbound = InboundMessage {
            message_id: msg.id.to_string(),
            gateway_id: self.gateway_id.clone(),
            external_id: channel_id.to_string(),
            conversation: ConversationId(format!("{}:{channel_id}", self.gateway_id)),
            group: msg
                .guild_id
                .map(|guild_id| GroupId(format!("discord:{}", guild_id.get()))),
            identity: None,
            profile: None,
            person: None,
            content,
            attachments,
            timestamp: msg.timestamp.unix_timestamp(),
            metadata: serde_json::json!({
                "platform": "discord",
                "channel_id": channel_id.to_string(),
                "guild_id": msg.guild_id.map(|guild_id| guild_id.get().to_string()),
                "author_id": msg.author.id.get().to_string(),
                "author_name": msg.author.name,
                "message_id": msg.id.get().to_string(),
            }),
        };

        if let Err(e) = self.inbound_tx.send(inbound).await {
            warn!(%e, gateway = %self.gateway_id, "failed to forward discord message");
        }
    }
}

fn token_from_vars(vars: &BTreeMap<String, Value>) -> anyhow::Result<String> {
    vars.get("bot_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("Discord bot token is required in gateway vars.bot_token"))
}

fn string_array_var(vars: &BTreeMap<String, Value>, key: &str) -> anyhow::Result<Vec<String>> {
    let Some(value) = vars.get(key) else {
        return Ok(vec![]);
    };
    let Some(values) = value.as_array() else {
        anyhow::bail!("{key} must be an array of strings");
    };

    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("{key} must contain only non-empty strings"))
        })
        .collect()
}

fn discord_setup_instructions() -> GatewaySetupInstructions {
    GatewaySetupInstructions::Text {
        title: "Connect Discord".into(),
        body: "Create a Discord bot, invite it to the server, enable the Message Content intent in the developer portal, then set bot_token in this gateway's vars.".into(),
    }
}

fn extract_message_content(msg: &Message) -> (String, Vec<MediaAttachment>) {
    let attachments = msg
        .attachments
        .iter()
        .map(|attachment| {
            discord_attachment(
                attachment.url.clone(),
                attachment.content_type.clone(),
                attachment.filename.clone(),
                u64::from(attachment.size),
            )
        })
        .collect();

    (msg.content.clone(), attachments)
}

fn discord_attachment(
    url: String,
    content_type: Option<String>,
    filename: String,
    size: u64,
) -> MediaAttachment {
    MediaAttachment {
        kind: media_kind_from_mime(content_type.as_deref()),
        asset_id: None,
        url: Some(url),
        mime: content_type,
        filename: Some(filename),
        size: Some(size),
    }
}

fn media_kind_from_mime(mime: Option<&str>) -> MediaKind {
    match mime.unwrap_or_default() {
        mime if mime.starts_with("image/") => MediaKind::Image,
        mime if mime.starts_with("video/") => MediaKind::Video,
        mime if mime.starts_with("audio/") => MediaKind::Audio,
        _ => MediaKind::File,
    }
}

fn is_missing_bot_token_error(error: &anyhow::Error) -> bool {
    error.to_string() == "Discord bot token is required in gateway vars.bot_token"
}

#[async_trait]
impl GatewayAdapter for DiscordAdapter {
    async fn connect(
        id: String,
        _db_path: String,
        vars: BTreeMap<String, Value>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        let config = match DiscordConfig::from_vars(&vars) {
            Ok(config) => config,
            Err(e) if is_missing_bot_token_error(&e) => {
                return Ok(Self::setup_required(id, gateway_event_tx, media_store).await);
            }
            Err(e) => return Err(e),
        };

        Self::connect_with_config(id, config, inbound_tx, gateway_event_tx, media_store).await
    }

    fn kind(&self) -> &str {
        "discord"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities {
                receive: vec![
                    GatewayContentKind::Text,
                    GatewayContentKind::Image,
                    GatewayContentKind::Video,
                    GatewayContentKind::Audio,
                    GatewayContentKind::File,
                ],
                send: vec![GatewayContentKind::Text],
            },
            composing: true,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        &self.id
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
        if !attachments.is_empty() {
            warn!("discord media sending not yet implemented, sending text only");
        }

        let http = self
            .http
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Discord gateway is not configured"))?;
        let channel_id = parse_channel_id(external_id)?;
        channel_id.say(http, content).await?;
        Ok(())
    }

    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()> {
        let http = self
            .http
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Discord gateway is not configured"))?;
        let channel_id = parse_channel_id(external_id)?;
        channel_id.broadcast_typing(http).await?;
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn parse_channel_id(external_id: &str) -> anyhow::Result<ChannelId> {
    let id = external_id
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid Discord channel id: {external_id}"))?;
    Ok(ChannelId::new(id))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            media_kind_from_mime(Some("image/png")),
            MediaKind::Image
        ));
        assert!(matches!(
            media_kind_from_mime(Some("video/mp4")),
            MediaKind::Video
        ));
        assert!(matches!(
            media_kind_from_mime(Some("audio/mpeg")),
            MediaKind::Audio
        ));
        assert!(matches!(media_kind_from_mime(None), MediaKind::File));
    }

    #[test]
    fn maps_discord_attachment_fields_without_dropping_metadata() {
        let attachment = discord_attachment(
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
}
