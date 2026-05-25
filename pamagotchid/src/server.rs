use crate::config::{Config, InferenceEntry, ProviderConfig};
use actor::core::{ActorBuilder, WakeEvent};
use actor::store::{SqliteConfig, SqliteStore};
use gateway::relay::RelayAdapter;
use gateway::GatewayRouter;
use inference::{
    InferenceEndpoint, InferenceRouter, InferenceRouterBuilder, OpenAiProvider, Retry,
    SamplingConfig,
};
use protocol::InboundMessage;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

pub async fn run(config: Config) -> anyhow::Result<()> {
    let relay = relay::RelayServer::listen(0).await?;
    let port = relay.port();
    info!(port, "relay server started");

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

    let mut gw_router = GatewayRouter::new();

    let relay_adapter = RelayAdapter::connect(port, "default", inbound_tx.clone()).await?;
    gw_router.register(Arc::new(relay_adapter));
    info!("relay gateway connected");

    let actor = ActorBuilder::new(store, Arc::new(router))
        .with_gateway(gw_router)
        .with_max_concurrency(config.max_concurrency)
        .with_max_turns(config.max_turns)
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

