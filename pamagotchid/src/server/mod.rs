use crate::config::{Config, InferenceEntry, ProviderConfig};
use actor::core::{ActorBuilder, ActorMetrics, MessageDeletedEvent, MessageEditedEvent, WakeEvent};
use actor::store::{
    ChannelRecord, EventInboxRecord, GatewayRecord, SpaceRecord, SqliteConfig, SqliteStore, Store,
};
use gateway::discord::DiscordAdapter;
use gateway::local::LocalAdapter;
use gateway::storage::{GatewayEntry, GatewayStore, gateway_data_dir};
use gateway::whatsapp::WhatsAppAdapter;
use gateway::{GatewayAdapter, GatewayRouter, GatewayRuntimeEvent};
use inference::{
    CodexProvider, InferenceEndpoint, InferenceProtocol, InferenceRouter, InferenceRouterBuilder,
    OpenAiProvider, Retry, SamplingConfig,
};
use media::MediaStore;
use protocol::{
    ClientRequest, ConversationId, GatewayKindView, GatewayVarKind, GatewayVarSpec, GatewayView,
    InboundEnvelope, InboundMessage, MediaAssetView, MediaKind, ServerEvent,
};
use relay::{ApiClientRequest, ApiServer, ApiServerHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

mod api;
mod debug_snapshot;
mod events;
mod gateways;
mod inference_router;
mod media_assets;
mod time;

use api::spawn_api_request_handler;
use debug_snapshot::debug_snapshot;
use events::{inbound_bridge, spawn_gateway_event_listener};
#[cfg(test)]
use events::{inbound_event_id, message_deleted_event_id, message_edited_event_id};
use gateways::{
    attach_configured_gateway, attach_configured_gateways, gateway_kind_view, gateway_view,
    is_supported_gateway_kind, supported_gateway_kinds, validate_gateway_vars,
};
use inference_router::build_inference_router;
use media_assets::{decode_base64, media_asset_view};
use time::{now_millis, now_secs};

const INBOUND_ACTOR_HANDOFF_TIMEOUT: Duration = Duration::from_millis(250);

struct GwApiContext {
    api_handle: ApiServerHandle,
    inbound_tx: mpsc::Sender<InboundEnvelope>,
    gw_router: Arc<GatewayRouter>,
    gateway_store: GatewayStore,
    gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
    media_store: Arc<MediaStore>,
    data_dir: PathBuf,
    store: Arc<SqliteStore>,
    metrics: Arc<ActorMetrics>,
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
    let inbound_tx = inbound_bridge(event_tx.clone(), store.clone());
    let (gateway_event_tx, gateway_event_rx) = mpsc::channel(256);
    let metrics = Arc::new(ActorMetrics::default());

    let gateway_store = GatewayStore::for_data_dir(&data_dir);
    let gw_router = Arc::new(GatewayRouter::new());
    gw_router.start_composing_sweep();
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
        store: store.clone(),
        metrics: metrics.clone(),
    };

    spawn_api_request_handler(api_request_rx, ctx);
    spawn_gateway_event_listener(
        gateway_event_rx,
        api_handle,
        event_tx.clone(),
        store.clone(),
    );

    let actor = ActorBuilder::new(store, Arc::new(router))
        .with_gateway(gw_router.clone())
        .with_media_store(media_store.clone())
        .with_max_concurrency(config.max_concurrency)
        .with_max_turns(config.max_turns)
        .with_retry(config.retry.max_attempts, config.retry.escalate_after)
        .with_event_channel(event_tx, event_rx)
        .with_metrics(metrics)
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

#[cfg(test)]
mod tests;
