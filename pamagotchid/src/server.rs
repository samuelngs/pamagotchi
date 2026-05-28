use crate::config::{Config, InferenceEntry, ProviderConfig};
use actor::core::{ActorBuilder, ActorMetrics, MessageDeletedEvent, MessageEditedEvent, WakeEvent};
use actor::store::{EventInboxRecord, SqliteConfig, SqliteStore, Store};
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
    InboundMessage, MediaAssetView, MediaKind, ServerEvent,
};
use relay::{ApiClientRequest, ApiServer, ApiServerHandle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const INBOUND_ACTOR_HANDOFF_TIMEOUT: Duration = Duration::from_millis(250);

struct GwApiContext {
    api_handle: ApiServerHandle,
    inbound_tx: mpsc::Sender<InboundMessage>,
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

fn inbound_bridge(
    event_tx: mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
) -> mpsc::Sender<InboundMessage> {
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<InboundMessage>(256);
    tokio::spawn(async move {
        while let Some(msg) = inbound_rx.recv().await {
            let event_id = persist_inbound_message_event(store.as_ref(), &msg).await;
            let permit = match event_id {
                Some(_) => {
                    match tokio::time::timeout(INBOUND_ACTOR_HANDOFF_TIMEOUT, event_tx.reserve())
                        .await
                    {
                        Ok(Ok(permit)) => permit,
                        Ok(Err(_)) => break,
                        Err(_) => {
                            warn!(
                                gateway = %msg.gateway_id,
                                message_id = %msg.message_id,
                                "actor event channel full; inbound message remains pending"
                            );
                            continue;
                        }
                    }
                }
                None => match event_tx.reserve().await {
                    Ok(permit) => permit,
                    Err(_) => break,
                },
            };
            if let Some(event_id) = event_id {
                match store.mark_event_fired(&event_id, now_secs()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        debug!(
                            event_id = %event_id,
                            message_id = %msg.message_id,
                            "inbound message event was already claimed"
                        );
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            %e,
                            event_id = %event_id,
                            message_id = %msg.message_id,
                            "failed to claim inbound message event; forwarding directly"
                        );
                    }
                }
            }
            permit.send(WakeEvent::Message(msg));
        }
    });
    inbound_tx
}

async fn persist_inbound_message_event(store: &dyn Store, msg: &InboundMessage) -> Option<String> {
    let now = now_secs();
    let event_id = inbound_event_id(msg);
    let record = EventInboxRecord {
        id: event_id.clone(),
        kind: "message".into(),
        payload: match serde_json::to_value(msg) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, message_id = %msg.message_id, "failed to serialize inbound message event");
                return None;
            }
        },
        status: "pending".into(),
        due_at: now,
        attempts: 0,
        dedupe_key: inbound_event_dedupe_key(msg),
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id),
        Err(e) => {
            warn!(%e, message_id = %msg.message_id, "failed to persist inbound message event");
            None
        }
    }
}

fn inbound_event_id(msg: &InboundMessage) -> String {
    if !msg.gateway_id.is_empty() && !msg.message_id.is_empty() {
        format!("event-inbound:{}:{}", msg.gateway_id, msg.message_id)
    } else {
        format!("event-inbound:{}:{}", now_millis(), rand::random::<u64>())
    }
}

fn inbound_event_dedupe_key(msg: &InboundMessage) -> Option<String> {
    (!msg.gateway_id.is_empty() && !msg.message_id.is_empty())
        .then(|| format!("inbound-message:{}:{}", msg.gateway_id, msg.message_id))
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
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
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
        ClientRequest::GetDebugSnapshot { request_id, limit } => {
            let snapshot = match debug_snapshot(
                ctx.store.as_ref(),
                ctx.metrics.as_ref(),
                limit.unwrap_or(20),
            )
            .await
            {
                Ok(snapshot) => snapshot,
                Err(e) => {
                    warn!(%e, "failed to build debug snapshot");
                    let _ = ctx
                        .api_handle
                        .send_to(
                            message.client_id,
                            ServerEvent::RequestError {
                                request_id: Some(request_id),
                                message: format!("failed to build debug snapshot: {e}"),
                            },
                        )
                        .await;
                    return;
                }
            };
            let _ = ctx
                .api_handle
                .send_to(
                    message.client_id,
                    ServerEvent::DebugSnapshot {
                        request_id,
                        snapshot,
                    },
                )
                .await;
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

async fn debug_snapshot(
    store: &dyn Store,
    metrics: &ActorMetrics,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let limit = limit.clamp(1, 100);
    let conversations = store.list_conversations().await?;
    let persons = store.list_persons().await?;
    let profiles = store.list_profiles().await?;
    let memories = store.debug_recent_memories(limit).await?;
    let memory_subjects = store.debug_memory_subjects(limit).await?;
    let profile_identity_links = store.debug_profile_identity_links(limit).await?;
    let person_profile_links = store.debug_person_profile_links(limit).await?;
    let groups = store.debug_groups(limit).await?;
    let intents = store.debug_active_intents(limit).await?;
    let review_outputs = store.debug_recent_review_outputs(limit).await?;
    let review_jobs = store.debug_recent_review_jobs(limit).await?;
    let raw_action_runs = store.debug_recent_action_runs(limit).await?;
    let mut action_runs = Vec::with_capacity(raw_action_runs.len());
    let mut action_traces = Vec::with_capacity(raw_action_runs.len());
    for run in &raw_action_runs {
        let run_value = serde_json::to_value(run)?;
        action_runs.push(redact_debug_trace_value(&run_value));
        let trace = serde_json::to_value(store.action_transcript(&run.action_id).await?)?;
        action_traces.push(redact_debug_trace_value(&trace));
    }
    let memory_mutations = store.debug_recent_memory_mutations(limit).await?;
    let failed_events = store.debug_recent_failed_events(limit).await?;
    let directives = store.list_directives().await?;
    let pending_claims = store.get_pending_claims().await?;

    Ok(serde_json::json!({
        "generated_at": now_secs(),
        "limit": limit,
        "metrics": metrics.snapshot(),
        "conversations": conversations,
        "persons": persons,
        "profiles": profiles,
        "profile_identity_links": profile_identity_links,
        "person_profile_links": person_profile_links,
        "groups": groups,
        "memories": memories,
        "memory_subjects": memory_subjects,
        "intents": intents,
        "review_outputs": review_outputs,
        "review_jobs": review_jobs,
        "action_runs": action_runs,
        "action_traces": action_traces,
        "memory_mutations": memory_mutations,
        "failed_events": failed_events,
        "directives": directives,
        "pending_identity_claims": pending_claims,
    }))
}

fn redact_debug_trace_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if should_redact_debug_trace_key(key) {
                        serde_json::Value::String("[redacted]".into())
                    } else {
                        redact_debug_trace_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_debug_trace_value).collect())
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                match serde_json::from_str::<serde_json::Value>(text) {
                    Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                        serde_json::Value::String(redact_debug_trace_value(&parsed).to_string())
                    }
                    _ => value.clone(),
                }
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn should_redact_debug_trace_key(key: &str) -> bool {
    matches!(
        key,
        "content"
            | "text"
            | "summary"
            | "comm_style"
            | "evidence_quote"
            | "reason"
            | "task"
            | "external_id"
            | "sender_external_id"
            | "reply_external_id"
            | "source_message_id"
            | "media_url"
            | "url"
            | "raw_arguments"
            | "error"
    )
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
    actor_event_tx: mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
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
                GatewayRuntimeEvent::TypingUpdate {
                    gateway_id,
                    conversation,
                    sender_external_id,
                    typing,
                } => {
                    if actor_event_tx
                        .send(WakeEvent::TypingUpdate {
                            conversation,
                            gateway_id,
                            sender_external_id,
                            typing,
                        })
                        .await
                        .is_err()
                    {
                        warn!("failed to forward gateway typing event to actor");
                    }
                }
                GatewayRuntimeEvent::MessageEdited {
                    gateway_id,
                    conversation,
                    message_id,
                    content,
                    edited_at,
                } => {
                    let edited = MessageEditedEvent {
                        conversation,
                        gateway_id,
                        message_id,
                        content,
                        edited_at,
                    };
                    let event_id = persist_message_edited_event(store.as_ref(), &edited).await;
                    let wake = WakeEvent::MessageEdited {
                        conversation: edited.conversation,
                        gateway_id: edited.gateway_id,
                        message_id: edited.message_id,
                        content: edited.content,
                        edited_at: edited.edited_at,
                    };
                    if !forward_persisted_gateway_event(
                        &actor_event_tx,
                        store.as_ref(),
                        event_id,
                        wake,
                        "gateway message edit",
                    )
                    .await
                    {
                        warn!("failed to forward gateway message edit event to actor");
                    }
                }
                GatewayRuntimeEvent::MessageDeleted {
                    gateway_id,
                    conversation,
                    message_id,
                    deleted_at,
                } => {
                    let deleted = MessageDeletedEvent {
                        conversation,
                        gateway_id,
                        message_id,
                        deleted_at,
                    };
                    let event_id = persist_message_deleted_event(store.as_ref(), &deleted).await;
                    let wake = WakeEvent::MessageDeleted {
                        conversation: deleted.conversation,
                        gateway_id: deleted.gateway_id,
                        message_id: deleted.message_id,
                        deleted_at: deleted.deleted_at,
                    };
                    if !forward_persisted_gateway_event(
                        &actor_event_tx,
                        store.as_ref(),
                        event_id,
                        wake,
                        "gateway message delete",
                    )
                    .await
                    {
                        warn!("failed to forward gateway message delete event to actor");
                    }
                }
            }
        }
    });
}

async fn persist_message_edited_event(
    store: &dyn Store,
    event: &MessageEditedEvent,
) -> Option<String> {
    let now = now_secs();
    let event_id = message_edited_event_id(event);
    let record = EventInboxRecord {
        id: event_id.clone(),
        kind: "message_edited".into(),
        payload: match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, message_id = %event.message_id, "failed to serialize message edit event");
                return None;
            }
        },
        status: "pending".into(),
        due_at: now,
        attempts: 0,
        dedupe_key: Some(message_edited_event_dedupe_key(event)),
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id),
        Err(e) => {
            warn!(%e, message_id = %event.message_id, "failed to persist message edit event");
            None
        }
    }
}

async fn persist_message_deleted_event(
    store: &dyn Store,
    event: &MessageDeletedEvent,
) -> Option<String> {
    let now = now_secs();
    let event_id = message_deleted_event_id(event);
    let record = EventInboxRecord {
        id: event_id.clone(),
        kind: "message_deleted".into(),
        payload: match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(%e, message_id = %event.message_id, "failed to serialize message delete event");
                return None;
            }
        },
        status: "pending".into(),
        due_at: now,
        attempts: 0,
        dedupe_key: Some(message_deleted_event_dedupe_key(event)),
        created_at: now,
        updated_at: now,
        fired_at: None,
        last_error: None,
    };
    match store.enqueue_event(&record).await {
        Ok(()) => Some(event_id),
        Err(e) => {
            warn!(%e, message_id = %event.message_id, "failed to persist message delete event");
            None
        }
    }
}

async fn forward_persisted_gateway_event(
    actor_event_tx: &mpsc::Sender<WakeEvent>,
    store: &dyn Store,
    event_id: Option<String>,
    wake: WakeEvent,
    context: &str,
) -> bool {
    let Some(event_id) = event_id else {
        return actor_event_tx.send(wake).await.is_ok();
    };

    let permit =
        match tokio::time::timeout(INBOUND_ACTOR_HANDOFF_TIMEOUT, actor_event_tx.reserve()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return false,
            Err(_) => {
                warn!(
                    event_id = %event_id,
                    context,
                    "actor event channel full; gateway event remains pending"
                );
                return true;
            }
        };

    match store.mark_event_fired(&event_id, now_secs()).await {
        Ok(true) => permit.send(wake),
        Ok(false) => {
            debug!(
                event_id = %event_id,
                context, "gateway event was already claimed"
            );
        }
        Err(e) => {
            warn!(
                %e,
                event_id = %event_id,
                context, "failed to claim gateway event; forwarding directly"
            );
            permit.send(wake);
        }
    }
    true
}

fn message_edited_event_id(event: &MessageEditedEvent) -> String {
    format!(
        "event-message-edit:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.edited_at
    )
}

fn message_edited_event_dedupe_key(event: &MessageEditedEvent) -> String {
    format!(
        "message-edit:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.edited_at
    )
}

fn message_deleted_event_id(event: &MessageDeletedEvent) -> String {
    format!(
        "event-message-delete:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.deleted_at
    )
}

fn message_deleted_event_dedupe_key(event: &MessageDeletedEvent) -> String {
    format!(
        "message-delete:{}:{}:{}:{}",
        event.gateway_id, event.conversation.0, event.message_id, event.deleted_at
    )
}

fn build_inference_router(config: &Config) -> anyhow::Result<InferenceRouter> {
    let mut builder = InferenceRouterBuilder::new();

    for entry in &config.inference {
        let (provider, model, sampling) = build_provider(entry)?;
        builder = builder.endpoint(InferenceEndpoint {
            protocol: provider,
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
) -> anyhow::Result<(InferenceProtocol, String, SamplingConfig)> {
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
            Ok((
                InferenceProtocol::OpenAiCompatible(Arc::new(retry)),
                opts.model.clone(),
                sampling,
            ))
        }
        ProviderConfig::Codex(opts) => {
            let provider = CodexProvider::new(opts.clone());
            let retry = Retry::new(provider, entry.max_retries)
                .with_base_delay(std::time::Duration::from_millis(entry.retry_delay_ms));
            Ok((
                InferenceProtocol::CodexAppServer(Arc::new(retry)),
                opts.model.clone(),
                SamplingConfig::default(),
            ))
        }
    }
}

#[cfg(test)]
mod tests;
