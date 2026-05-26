use crate::config::{Config, InferenceEntry, ProviderConfig};
use actor::core::{ActorBuilder, WakeEvent};
use actor::store::{SqliteConfig, SqliteStore};
use gateway::discord::DiscordAdapter;
use gateway::local::LocalAdapter;
use gateway::storage::{GatewayEntry, GatewayStore, gateway_data_dir};
use gateway::whatsapp::WhatsAppAdapter;
use gateway::{GatewayAdapter, GatewayRouter, GatewayRuntimeEvent};
use inference::{
    InferenceEndpoint, InferenceRouter, InferenceRouterBuilder, OpenAiProvider, Retry,
    SamplingConfig,
};
use media::MediaStore;
use protocol::{
    ClientRequest, ConversationId, GatewayKindView, GatewayVarKind, GatewayVarSpec, GatewayView,
    InboundMessage, MediaAssetView, MediaKind, ServerEvent,
};
use relay::{ApiClientRequest, ApiServer, ApiServerHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

struct GwApiContext {
    api_handle: ApiServerHandle,
    inbound_tx: mpsc::Sender<InboundMessage>,
    gw_router: Arc<GatewayRouter>,
    gateway_store: GatewayStore,
    gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
    media_store: Arc<MediaStore>,
    data_dir: PathBuf,
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    let (api, api_request_rx) = ApiServer::listen(0).await?;
    let port = api.port();
    let api_handle = api.handle();
    info!(port, "api server started");

    let data_dir = config.data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let pid_path = data_dir.join("pamagotchid.pid");
    std::fs::write(&pid_path, format!("{}\n{port}", std::process::id()))?;
    info!(?pid_path, "pid file written");

    let router = build_inference_router(&config)?;
    info!(
        inference_count = config.inference.len(),
        "inference router built"
    );

    let store = Arc::new(SqliteStore::open(SqliteConfig {
        path: config.store_path().to_string_lossy().to_string(),
        ..Default::default()
    })?);

    let (event_tx, event_rx) = mpsc::channel(256);
    let inbound_tx = inbound_bridge(event_tx.clone());
    let (gateway_event_tx, gateway_event_rx) = mpsc::channel(256);

    let gateway_store = GatewayStore::for_data_dir(&data_dir);
    let gw_router = Arc::new(GatewayRouter::new());
    let media_store = Arc::new(MediaStore::open(data_dir.join("media"))?);
    info!(path = %media_store.root().display(), "media store opened");

    let local_adapter = LocalAdapter::new(api_handle.clone());
    gw_router.register(Arc::new(local_adapter));
    info!("local api gateway connected");

    attach_configured_gateways(
        &gw_router,
        &data_dir,
        inbound_tx.clone(),
        gateway_event_tx.clone(),
        media_store.clone(),
    )
    .await?;

    let ctx = GwApiContext {
        api_handle: api_handle.clone(),
        inbound_tx: inbound_tx.clone(),
        gw_router: gw_router.clone(),
        gateway_store,
        gateway_event_tx: gateway_event_tx.clone(),
        media_store: media_store.clone(),
        data_dir: data_dir.clone(),
    };

    spawn_api_request_handler(api_request_rx, ctx);
    spawn_gateway_event_listener(gateway_event_rx, api_handle);

    let actor = ActorBuilder::new(store, Arc::new(router))
        .with_gateway(gw_router.clone())
        .with_max_concurrency(config.max_concurrency)
        .with_max_turns(config.max_turns)
        .with_retry(config.retry.max_attempts, config.retry.escalate_after)
        .with_event_channel(event_tx, event_rx)
        .build()
        .await?;

    info!("actor started");
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");

    if let Err(e) = actor.shutdown().await {
        error!(%e, "actor shutdown error");
    }

    let _ = std::fs::remove_file(&pid_path);
    info!("pamagotchi stopped");
    Ok(())
}

async fn attach_configured_gateways(
    gw_router: &Arc<GatewayRouter>,
    data_dir: &std::path::Path,
    inbound_tx: mpsc::Sender<InboundMessage>,
    gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
    media_store: Arc<MediaStore>,
) -> anyhow::Result<()> {
    let store = GatewayStore::for_data_dir(data_dir);
    let settings = store.load_or_create()?;

    for entry in &settings.gateway {
        attach_configured_gateway(
            gw_router,
            data_dir,
            entry,
            inbound_tx.clone(),
            gateway_event_tx.clone(),
            media_store.clone(),
        )
        .await?;
    }

    info!(
        path = %store.path().display(),
        gateway_count = settings.gateway.len(),
        "gateway settings loaded"
    );
    Ok(())
}

async fn attach_configured_gateway(
    gw_router: &Arc<GatewayRouter>,
    data_dir: &std::path::Path,
    entry: &GatewayEntry,
    inbound_tx: mpsc::Sender<InboundMessage>,
    gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
    media_store: Arc<MediaStore>,
) -> anyhow::Result<()> {
    let gateway_dir = gateway_data_dir(data_dir, &entry.id);
    std::fs::create_dir_all(&gateway_dir)?;

    match entry.kind.as_str() {
        "whatsapp" => {
            let db_path = gateway_dir.join("whatsapp.db");
            let db_path = db_path.to_string_lossy().to_string();
            let adapter = WhatsAppAdapter::connect(
                entry.id.clone(),
                db_path,
                entry.vars.clone(),
                inbound_tx,
                gateway_event_tx,
                media_store,
            )
            .await?;
            gw_router.register(Arc::new(adapter));
            info!(
                gateway = %entry.id,
                kind = %entry.kind,
                data_dir = %gateway_dir.display(),
                "gateway connected"
            );
        }
        "discord" => {
            let db_path = gateway_dir.join("discord.db");
            let db_path = db_path.to_string_lossy().to_string();
            let adapter = DiscordAdapter::connect(
                entry.id.clone(),
                db_path,
                entry.vars.clone(),
                inbound_tx,
                gateway_event_tx,
                media_store,
            )
            .await?;
            gw_router.register(Arc::new(adapter));
            info!(
                gateway = %entry.id,
                kind = %entry.kind,
                data_dir = %gateway_dir.display(),
                "gateway connected"
            );
        }
        kind => {
            warn!(
                gateway = %entry.id,
                kind,
                "configured gateway kind is not supported yet"
            );
        }
    }

    Ok(())
}

fn inbound_bridge(event_tx: mpsc::Sender<WakeEvent>) -> mpsc::Sender<InboundMessage> {
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<InboundMessage>(256);
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            if event_tx.send(WakeEvent::Message(msg)).await.is_err() {
                break;
            }
        }
    });
    inbound_tx
}

fn spawn_api_request_handler(
    mut api_request_rx: mpsc::Receiver<ApiClientRequest>,
    ctx: GwApiContext,
) {
    tokio::spawn(async move {
        while let Some(message) = api_request_rx.recv().await {
            handle_api_request(message, &ctx).await;
        }
    });
}

async fn handle_api_request(message: ApiClientRequest, ctx: &GwApiContext) {
    match message.request {
        ClientRequest::Subscribe { .. } => {}
        ClientRequest::SendChatMessage { content } => {
            let inbound = InboundMessage {
                message_id: format!("local-{}", now_millis()),
                gateway_id: "relay".into(),
                external_id: "local".into(),
                conversation: ConversationId("relay:local".into()),
                group: None,
                identity: None,
                profile: None,
                person: None,
                content,
                attachments: Vec::new(),
                timestamp: now_secs(),
                metadata: serde_json::Value::Null,
            };
            if let Err(e) = ctx.inbound_tx.send(inbound).await {
                warn!(%e, client_id = message.client_id, "failed to forward api chat message");
            }
        }
        ClientRequest::CreateMediaAsset {
            request_id,
            kind,
            data_base64,
            mime,
            filename,
        } => {
            let Some(kind) = MediaKind::parse(&kind) else {
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("unknown media kind: {kind}"),
                        },
                    )
                    .await;
                return;
            };

            let bytes = match decode_base64(&data_base64) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("invalid base64 media data: {e}"),
                            },
                        )
                        .await;
                    return;
                }
            };

            let new_asset = media::NewMediaAsset {
                kind,
                mime,
                filename,
                metadata: serde_json::json!({
                    "source": "api",
                    "client_id": message.client_id,
                }),
            };

            match ctx.media_store.put_bytes(&bytes, new_asset) {
                Ok(asset) => {
                    let view = media_asset_view(asset);
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::MediaAssetCreated {
                                request_id,
                                asset: view,
                            },
                        )
                        .await;
                }
                Err(e) => {
                    warn!(%e, "failed to create media asset");
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("failed to create media asset: {e}"),
                            },
                        )
                        .await;
                }
            }
        }
        ClientRequest::ListGateways { request_id } => {
            let settings = match ctx.gateway_store.load() {
                Ok(s) => s,
                Err(e) => {
                    warn!(%e, "failed to load gateway settings");
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("failed to load gateways: {e}"),
                            },
                        )
                        .await;
                    return;
                }
            };

            let gateways = settings
                .gateway
                .iter()
                .map(|entry| gateway_view(entry, &ctx.gw_router))
                .collect();

            let _ = ctx
                .api_handle
                .send_to(
                    message.client_id,
                    ServerEvent::GatewayList {
                        request_id,
                        gateways,
                    },
                )
                .await;
        }
        ClientRequest::ListAvailableGateways { request_id } => {
            let gateways = supported_gateway_kinds()
                .iter()
                .map(|kind| gateway_kind_view(kind))
                .collect();

            let _ = ctx
                .api_handle
                .send_to(
                    message.client_id,
                    ServerEvent::AvailableGatewayList {
                        request_id,
                        gateways,
                    },
                )
                .await;
        }
        ClientRequest::AddGateway {
            request_id,
            kind,
            vars,
        } => {
            if !is_supported_gateway_kind(&kind) {
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("unsupported gateway kind: {kind}"),
                        },
                    )
                    .await;
                return;
            }

            let entry_vars: std::collections::BTreeMap<String, serde_json::Value> =
                serde_json::from_value(vars.clone()).unwrap_or_default();

            let entry = match ctx.gateway_store.add(&kind, entry_vars) {
                Ok(e) => e,
                Err(e) => {
                    warn!(%e, kind, "failed to add gateway to store");
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("failed to add gateway: {e}"),
                            },
                        )
                        .await;
                    return;
                }
            };

            if let Err(e) = attach_configured_gateway(
                &ctx.gw_router,
                &ctx.data_dir,
                &entry,
                ctx.inbound_tx.clone(),
                ctx.gateway_event_tx.clone(),
                ctx.media_store.clone(),
            )
            .await
            {
                warn!(%e, gateway = %entry.id, "failed to start added gateway");
                let _ = ctx.gateway_store.remove(&entry.id);
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("failed to start gateway: {e}"),
                        },
                    )
                    .await;
                return;
            }

            let gateway = gateway_view(&entry, &ctx.gw_router);
            ctx.api_handle
                .broadcast(ServerEvent::GatewayAdded { gateway })
                .await;
            info!(gateway = %entry.id, kind = %entry.kind, "gateway added and broadcast");

            let _ = ctx
                .api_handle
                .send_to(message.client_id, ServerEvent::RequestOk { request_id })
                .await;
        }
        ClientRequest::RemoveGateway { request_id, id } => match ctx.gateway_store.remove(&id) {
            Ok(Some(_)) => {
                ctx.gw_router.unregister(&id);
                ctx.api_handle
                    .broadcast(ServerEvent::GatewayRemoved { id: id.clone() })
                    .await;
                let _ = ctx
                    .api_handle
                    .send_to(message.client_id, ServerEvent::RequestOk { request_id })
                    .await;
            }
            Ok(None) => {
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("gateway not found: {id}"),
                        },
                    )
                    .await;
            }
            Err(e) => {
                warn!(%e, gateway = %id, "failed to remove gateway");
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("failed to remove gateway: {e}"),
                        },
                    )
                    .await;
            }
        },
        ClientRequest::RestartGateway { request_id, id } => {
            let settings = match ctx.gateway_store.load() {
                Ok(settings) => settings,
                Err(e) => {
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("failed to load gateways: {e}"),
                            },
                        )
                        .await;
                    return;
                }
            };
            let Some(entry) = settings
                .gateway
                .iter()
                .find(|entry| entry.id == id)
                .cloned()
            else {
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("gateway not found: {id}"),
                        },
                    )
                    .await;
                return;
            };

            ctx.gw_router.unregister(&id);
            if let Err(e) = attach_configured_gateway(
                &ctx.gw_router,
                &ctx.data_dir,
                &entry,
                ctx.inbound_tx.clone(),
                ctx.gateway_event_tx.clone(),
                ctx.media_store.clone(),
            )
            .await
            {
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("failed to restart gateway: {e}"),
                        },
                    )
                    .await;
                return;
            }

            ctx.api_handle
                .broadcast(ServerEvent::GatewayUpdated {
                    gateway: gateway_view(&entry, &ctx.gw_router),
                })
                .await;
            let _ = ctx
                .api_handle
                .send_to(message.client_id, ServerEvent::RequestOk { request_id })
                .await;
        }
        ClientRequest::UpdateGatewayVars {
            request_id,
            id,
            vars,
        } => {
            let entry_vars: std::collections::BTreeMap<String, serde_json::Value> =
                serde_json::from_value(vars.clone()).unwrap_or_default();

            if let Err(e) = validate_gateway_vars(&entry_vars) {
                let _ = ctx
                    .api_handle
                    .send_to(
                        message.client_id,
                        ServerEvent::RequestError {
                            request_id: Some(request_id),
                            message: format!("invalid gateway vars: {e}"),
                        },
                    )
                    .await;
                return;
            }

            let entry = match ctx.gateway_store.update_vars(&id, entry_vars) {
                Ok(Some(entry)) => entry,
                Ok(None) => {
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("gateway not found: {id}"),
                            },
                        )
                        .await;
                    return;
                }
                Err(e) => {
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("failed to update gateway vars: {e}"),
                            },
                        )
                        .await;
                    return;
                }
            };

            ctx.gw_router.unregister(&id);
            if let Err(e) = attach_configured_gateway(
                &ctx.gw_router,
                &ctx.data_dir,
                &entry,
                ctx.inbound_tx.clone(),
                ctx.gateway_event_tx.clone(),
                ctx.media_store.clone(),
            )
            .await
            {
                warn!(%e, gateway = %entry.id, "failed to restart gateway after vars update");
            }

            ctx.api_handle
                .broadcast(ServerEvent::GatewayUpdated {
                    gateway: gateway_view(&entry, &ctx.gw_router),
                })
                .await;
            let _ = ctx
                .api_handle
                .send_to(message.client_id, ServerEvent::RequestOk { request_id })
                .await;
        }
    }
}

fn decode_base64(input: &str) -> anyhow::Result<Vec<u8>> {
    use base64::Engine as _;

    let data = input.split_once(',').map(|(_, data)| data).unwrap_or(input);
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data))
        .map_err(Into::into)
}

fn media_asset_view(asset: media::MediaAsset) -> MediaAssetView {
    MediaAssetView {
        id: asset.id,
        kind: asset.kind,
        mime: asset.mime,
        filename: asset.filename,
        size: asset.size,
        sha256: asset.sha256,
    }
}

fn gateway_view(entry: &GatewayEntry, gw_router: &GatewayRouter) -> GatewayView {
    let adapter = gw_router.get(&entry.id);
    let (connection_state, setup_instructions) = if let Some(ref adapter) = adapter {
        (adapter.connection_state(), adapter.setup_instructions())
    } else {
        (gateway::GatewayConnectionState::Disconnected, None)
    };

    GatewayView {
        id: entry.id.clone(),
        kind: entry.kind.clone(),
        vars: serde_json::to_value(&entry.vars).unwrap_or_default(),
        connection_state,
        setup_instructions,
    }
}

fn is_supported_gateway_kind(kind: &str) -> bool {
    supported_gateway_kinds().contains(&kind)
}

fn supported_gateway_kinds() -> &'static [&'static str] {
    &["whatsapp", "discord"]
}

fn gateway_kind_view(kind: &str) -> GatewayKindView {
    GatewayKindView {
        kind: kind.to_string(),
        vars: gateway_var_specs(kind),
    }
}

fn gateway_var_specs(kind: &str) -> Vec<GatewayVarSpec> {
    match kind {
        "discord" => vec![
            GatewayVarSpec {
                key: "bot_token".into(),
                label: "Bot token".into(),
                kind: GatewayVarKind::String,
                required: true,
                secret: true,
                default: None,
                help: Some("Discord bot token from the Discord developer portal.".into()),
            },
            GatewayVarSpec {
                key: "allowed_channel_ids".into(),
                label: "Allowed channel IDs".into(),
                kind: GatewayVarKind::StringList,
                required: false,
                secret: false,
                default: Some(serde_json::json!([])),
                help: Some(
                    "Optional Discord channel ID allowlist. Empty allows all channels.".into(),
                ),
            },
            GatewayVarSpec {
                key: "ignore_bots".into(),
                label: "Ignore bots".into(),
                kind: GatewayVarKind::Bool,
                required: false,
                secret: false,
                default: Some(serde_json::json!(true)),
                help: Some("Ignore messages from Discord bot users.".into()),
            },
        ],
        _ => vec![],
    }
}

fn validate_gateway_vars(
    vars: &std::collections::BTreeMap<String, serde_json::Value>,
) -> anyhow::Result<()> {
    for (key, value) in vars {
        match (key.as_str(), value) {
            ("bot_token", serde_json::Value::String(_)) => {}
            ("allowed_channel_ids", serde_json::Value::Array(values)) => {
                if !values.iter().all(|value| value.as_str().is_some()) {
                    anyhow::bail!("allowed_channel_ids must be an array of strings");
                }
            }
            ("ignore_bots", serde_json::Value::Bool(_)) => {}
            ("bot_token", _) => anyhow::bail!("bot_token must be a string"),
            ("allowed_channel_ids", _) => anyhow::bail!("allowed_channel_ids must be an array"),
            ("ignore_bots", _) => anyhow::bail!("ignore_bots must be a boolean"),
            _ => {}
        }
    }
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn spawn_gateway_event_listener(
    mut gateway_event_rx: mpsc::Receiver<GatewayRuntimeEvent>,
    api_handle: ApiServerHandle,
) {
    tokio::spawn(async move {
        while let Some(event) = gateway_event_rx.recv().await {
            match event {
                GatewayRuntimeEvent::ConnectionStateChanged { gateway_id, state } => {
                    info!(gateway = %gateway_id, ?state, "gateway connection state changed");
                    api_handle
                        .broadcast(ServerEvent::GatewayConnectionStateChanged {
                            id: gateway_id,
                            state,
                        })
                        .await;
                }
                GatewayRuntimeEvent::SetupInstructionsChanged { gateway_id, setup } => {
                    info!(
                        gateway = %gateway_id,
                        has_setup = setup.is_some(),
                        "gateway setup instructions changed"
                    );
                    api_handle
                        .broadcast(ServerEvent::GatewaySetupInstructionsChanged {
                            id: gateway_id,
                            setup,
                        })
                        .await;
                }
            }
        }
    });
}

fn build_inference_router(config: &Config) -> anyhow::Result<InferenceRouter> {
    let mut builder = InferenceRouterBuilder::new();

    for entry in &config.inference {
        let (provider, model, sampling) = build_provider(entry)?;
        builder = builder.endpoint(InferenceEndpoint {
            provider,
            model,
            sampling,
            capabilities: entry.capabilities.clone(),
            reasoning: entry.reasoning,
        });
    }

    builder.build()
}

fn build_provider(
    entry: &InferenceEntry,
) -> anyhow::Result<(Arc<dyn inference::Provider>, String, SamplingConfig)> {
    match &entry.provider {
        ProviderConfig::OpenAi(opts) => {
            let base_url = opts
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let api_key = match opts.api_key.as_deref() {
                Some(key) => key,
                None if base_url.contains("api.openai.com") => {
                    anyhow::bail!("api_key required for {base_url}");
                }
                None => "",
            };
            let provider = OpenAiProvider::new(base_url, api_key)
                .with_tool_choice_required(opts.tool_choice_required);
            let retry = Retry::new(provider, entry.max_retries)
                .with_base_delay(std::time::Duration::from_millis(entry.retry_delay_ms));
            let sampling = SamplingConfig {
                temperature: opts.temperature,
                top_p: opts.top_p,
                top_k: opts.top_k,
                min_p: opts.min_p,
            };
            Ok((Arc::new(retry), opts.model.clone(), sampling))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_base64_accepts_plain_and_data_url_inputs() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(
            decode_base64("data:image/png;base64,aGVsbG8=").unwrap(),
            b"hello"
        );
    }
}
