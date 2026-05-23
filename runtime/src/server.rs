use crate::config::{ActorEntry, Config, PlatformEntry};
use actor::core::{Actor, ActorBuilder};
use actor::identity::PersonId;
use actor::llm::{OpenAiProvider, Provider};
use actor::platform::whatsapp::WhatsAppAdapter;
use actor::platform::PlatformRouter;
use actor::store::{ActorConfig, SqliteConfig, SqliteStore};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub async fn run(config: Config) -> anyhow::Result<()> {
    let data_dir = config.data_dir();
    let mut actors: Vec<(String, Actor)> = Vec::new();

    for entry in &config.actors {
        match start_actor(entry, &data_dir).await {
            Ok(Some(actor)) => actors.push((entry.name.clone(), actor)),
            Ok(None) => {}
            Err(e) => {
                error!(actor = %entry.name, %e, "failed to start actor");
                return Err(e);
            }
        }
    }

    if actors.is_empty() {
        warn!("no actors running");
    }

    info!(count = actors.len(), "all actors running");
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");

    for (name, actor) in actors {
        info!(name = %name, "shutting down actor");
        if let Err(e) = actor.shutdown().await {
            error!(name = %name, %e, "actor shutdown error");
        }
    }

    info!("pamagotchi stopped");
    Ok(())
}

async fn start_actor(entry: &ActorEntry, data_dir: &Path) -> anyhow::Result<Option<Actor>> {
    let provider_config = match entry.provider.as_ref() {
        Some(p) => p,
        None => {
            warn!(actor = %entry.name, "no provider configured, skipping");
            return Ok(None);
        }
    };

    let provider = build_provider(&provider_config.chat)?;
    let model = provider_config.chat.model();
    let sampling = provider_config.chat.sampling();

    let actor_dir = entry.actor_data_dir(data_dir);
    std::fs::create_dir_all(&actor_dir)?;

    let store = Arc::new(SqliteStore::open(SqliteConfig {
        path: entry.store_path(data_dir).to_string_lossy().to_string(),
        ..Default::default()
    })?);

    let (event_tx, event_rx) = mpsc::channel(256);
    let router = build_platforms(&entry.platforms, &actor_dir, &entry.name, &event_tx).await?;

    let actor_config = ActorConfig {
        name: entry.name.clone(),
        description: String::new(),
        owner: PersonId(String::new()),
    };

    let actor = ActorBuilder::new(actor_config, store, provider)
        .with_model(model)
        .with_sampling(sampling)
        .with_platform(router)
        .with_max_concurrency(entry.max_concurrency)
        .with_event_channel(event_tx, event_rx)
        .build()
        .await?;

    info!(name = %entry.name, id = %entry.id, "actor started");
    Ok(Some(actor))
}

fn build_provider(entry: &crate::config::ProviderEntry) -> anyhow::Result<Arc<dyn Provider>> {
    match entry {
        crate::config::ProviderEntry::OpenAi {
            base_url, api_key, ..
        } => {
            let base_url = base_url.as_deref().unwrap_or("https://api.openai.com/v1");
            let api_key = match api_key.as_deref() {
                Some(key) => key,
                None if base_url.contains("api.openai.com") => {
                    anyhow::bail!("api_key required for {base_url}");
                }
                None => "",
            };
            Ok(Arc::new(OpenAiProvider::new(base_url, api_key)))
        }
    }
}

async fn build_platforms(
    platforms: &[PlatformEntry],
    actor_dir: &Path,
    actor_name: &str,
    event_tx: &mpsc::Sender<actor::core::WakeEvent>,
) -> anyhow::Result<PlatformRouter> {
    let mut router = PlatformRouter::new();

    for platform in platforms {
        match platform {
            PlatformEntry::WhatsApp {} => {
                let db_path = platform.db_path(actor_dir);
                let adapter =
                    WhatsAppAdapter::connect(&db_path.to_string_lossy(), event_tx.clone()).await?;
                info!(actor = %actor_name, "whatsapp connected");
                router.register(Arc::new(adapter));
            }
        }
    }

    Ok(router)
}
