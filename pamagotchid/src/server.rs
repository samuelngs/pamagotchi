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
mod tests {
    use super::*;
    use actor::identity::{Group, GroupContext, Identity, Person, PersonProfileStatus, Profile};
    use actor::store::{
        ActionMessageRecord, ActionRunRecord, ActionTurnRecord, Memory, MemoryKind, MemorySource,
        MemorySubject, OutboundDeliveryRecord, Store, ToolCallRecord,
    };
    use protocol::{GroupId, IdentityId, MemoryId, PersonId, ProfileId};

    #[test]
    fn decode_base64_accepts_plain_and_data_url_inputs() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(
            decode_base64("data:image/png;base64,aGVsbG8=").unwrap(),
            b"hello"
        );
    }

    #[tokio::test]
    async fn inbound_bridge_persists_claims_and_forwards_message() {
        let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let inbound_tx = inbound_bridge(event_tx, store_dyn);
        let msg = test_inbound("bridge-msg-1");

        inbound_tx.send(msg.clone()).await.unwrap();

        match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WakeEvent::Message(forwarded) => assert_eq!(forwarded.message_id, msg.message_id),
            _ => panic!("expected forwarded inbound message"),
        }

        assert!(
            !store
                .mark_event_fired(&inbound_event_id(&msg), now_secs())
                .await
                .unwrap()
        );
        assert!(
            store
                .due_events(now_secs() + 1, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn inbound_bridge_suppresses_duplicate_claimed_source_message() {
        let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let inbound_tx = inbound_bridge(event_tx, store_dyn);
        let msg = test_inbound("bridge-msg-duplicate");

        inbound_tx.send(msg.clone()).await.unwrap();
        inbound_tx.send(msg).await.unwrap();

        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), event_rx.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn inbound_bridge_leaves_pending_event_when_actor_channel_is_closed() {
        let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (event_tx, event_rx) = mpsc::channel(1);
        drop(event_rx);
        let inbound_tx = inbound_bridge(event_tx, store_dyn);
        let msg = test_inbound("bridge-msg-pending");
        let event_id = inbound_event_id(&msg);

        inbound_tx.send(msg).await.unwrap();

        let mut due = Vec::new();
        for _ in 0..20 {
            due = store.due_events(now_secs() + 1, 10).await.unwrap();
            if !due.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, event_id);
        assert_eq!(due[0].status, "pending");
    }

    #[tokio::test]
    async fn inbound_bridge_leaves_overflow_pending_and_keeps_accepting_messages() {
        let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let inbound_tx = inbound_bridge(event_tx, store_dyn);
        let first = test_inbound("bridge-overflow-1");
        let mut second = test_inbound("bridge-overflow-2");
        second.conversation = ConversationId("relay:overflow-2".into());
        let mut third = test_inbound("bridge-overflow-3");
        third.conversation = ConversationId("relay:overflow-3".into());

        inbound_tx.send(first.clone()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if event_rx.len() == 1 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        inbound_tx.send(second.clone()).await.unwrap();
        inbound_tx.send(third.clone()).await.unwrap();

        let mut pending = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                pending = store.pending_events_by_kind("message", 10).await.unwrap();
                if pending.len() >= 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let pending_ids = pending
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>();
        assert!(pending_ids.contains(&inbound_event_id(&second).as_str()));
        assert!(pending_ids.contains(&inbound_event_id(&third).as_str()));

        match event_rx.try_recv().unwrap() {
            WakeEvent::Message(forwarded) => assert_eq!(forwarded.message_id, first.message_id),
            _ => panic!("expected first inbound message to be forwarded"),
        }
    }

    #[tokio::test]
    async fn gateway_event_listener_persists_and_forwards_message_revisions() {
        let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (api, _api_rx) = ApiServer::listen(0).await.unwrap();
        let (gateway_event_tx, gateway_event_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        spawn_gateway_event_listener(gateway_event_rx, api.handle(), event_tx, store_dyn);

        gateway_event_tx
            .send(GatewayRuntimeEvent::MessageEdited {
                gateway_id: "relay".into(),
                conversation: ConversationId("relay:local".into()),
                message_id: "revision-msg-1".into(),
                content: "edited content".into(),
                edited_at: 1100,
            })
            .await
            .unwrap();
        gateway_event_tx
            .send(GatewayRuntimeEvent::MessageDeleted {
                gateway_id: "relay".into(),
                conversation: ConversationId("relay:local".into()),
                message_id: "revision-msg-2".into(),
                deleted_at: 1200,
            })
            .await
            .unwrap();

        match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WakeEvent::MessageEdited {
                gateway_id,
                message_id,
                content,
                edited_at,
                ..
            } => {
                assert_eq!(gateway_id, "relay");
                assert_eq!(message_id, "revision-msg-1");
                assert_eq!(content, "edited content");
                assert_eq!(edited_at, 1100);
            }
            _ => panic!("expected message edit event"),
        }
        match tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WakeEvent::MessageDeleted {
                gateway_id,
                message_id,
                deleted_at,
                ..
            } => {
                assert_eq!(gateway_id, "relay");
                assert_eq!(message_id, "revision-msg-2");
                assert_eq!(deleted_at, 1200);
            }
            _ => panic!("expected message delete event"),
        }

        let edited = MessageEditedEvent {
            conversation: ConversationId("relay:local".into()),
            gateway_id: "relay".into(),
            message_id: "revision-msg-1".into(),
            content: "edited content".into(),
            edited_at: 1100,
        };
        let deleted = MessageDeletedEvent {
            conversation: ConversationId("relay:local".into()),
            gateway_id: "relay".into(),
            message_id: "revision-msg-2".into(),
            deleted_at: 1200,
        };
        assert!(
            !store
                .mark_event_fired(&message_edited_event_id(&edited), now_secs())
                .await
                .unwrap()
        );
        assert!(
            !store
                .mark_event_fired(&message_deleted_event_id(&deleted), now_secs())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn gateway_event_listener_leaves_revision_pending_when_actor_channel_is_closed() {
        let store = Arc::new(actor::store::SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (api, _api_rx) = ApiServer::listen(0).await.unwrap();
        let (gateway_event_tx, gateway_event_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(1);
        drop(event_rx);
        spawn_gateway_event_listener(gateway_event_rx, api.handle(), event_tx, store_dyn);

        let edited = MessageEditedEvent {
            conversation: ConversationId("relay:local".into()),
            gateway_id: "relay".into(),
            message_id: "pending-revision-msg".into(),
            content: "edited content".into(),
            edited_at: 1100,
        };
        gateway_event_tx
            .send(GatewayRuntimeEvent::MessageEdited {
                gateway_id: edited.gateway_id.clone(),
                conversation: edited.conversation.clone(),
                message_id: edited.message_id.clone(),
                content: edited.content.clone(),
                edited_at: edited.edited_at,
            })
            .await
            .unwrap();

        let mut due = Vec::new();
        for _ in 0..20 {
            due = store.due_events(now_secs() + 1, 10).await.unwrap();
            if !due.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, message_edited_event_id(&edited));
        assert_eq!(due[0].kind, "message_edited");
        assert_eq!(due[0].status, "pending");
    }

    fn test_inbound(message_id: &str) -> InboundMessage {
        InboundMessage {
            message_id: message_id.into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: ConversationId("relay:local".into()),
            group: None,
            identity: None,
            profile: None,
            person: None,
            content: "hello".into(),
            attachments: Vec::new(),
            timestamp: now_secs(),
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn debug_snapshot_includes_memory_mutations() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert!(snapshot["memory_mutations"].is_array());
    }

    #[tokio::test]
    async fn debug_snapshot_includes_memory_subject_index() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        let profile = ProfileId("profile-debug".into());
        let mut memory = Memory {
            id: MemoryId("memory-subject-debug".into()),
            kind: MemoryKind::Semantic,
            content: "Debug profile prefers concise summaries.".into(),
            source: MemorySource::External,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            created_at: 1000,
            accessed_at: 1000,
            ..Memory::default()
        };
        memory.embedding = Some(vec![0.1, 0.2, 0.3, 0.4]);
        store.store_memory(&memory).await.unwrap();

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(snapshot["memory_subjects"][0]["subject_id"], profile.0);
        assert_eq!(snapshot["memory_subjects"][0]["memory_count"], 1);
        assert_eq!(
            snapshot["memory_subjects"][0]["latest_memory_ids"][0],
            "memory-subject-debug"
        );
    }

    #[tokio::test]
    async fn debug_snapshot_includes_identity_link_evidence() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        let identity = Identity {
            id: IdentityId("identity-debug".into()),
            gateway_id: "discord".into(),
            external_id: "debug-user".into(),
            display_name: Some("Debug User".into()),
            metadata: None,
            created_at: 1000,
            last_seen_at: 1000,
        };
        let profile = Profile {
            id: ProfileId("profile-debug".into()),
            display_name: Some("Debug User".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
            created_at: 1000,
            updated_at: 1000,
        };
        let person = Person {
            id: PersonId("person-debug".into()),
            name: Some("Debug User".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        };
        store.add_identity(&identity).await.unwrap();
        store.add_profile(&profile).await.unwrap();
        store.add_person(&person).await.unwrap();
        store
            .link_identity_to_profile(
                &identity.id,
                &profile.id,
                0.91,
                Some(&serde_json::json!({"message_id": "msg-profile-link"})),
            )
            .await
            .unwrap();
        store
            .attach_profile_to_person(
                &profile.id,
                &person.id,
                PersonProfileStatus::Verified,
                0.97,
                Some(&serde_json::json!({"message_id": "msg-person-link"})),
            )
            .await
            .unwrap();

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(
            snapshot["profile_identity_links"][0]["evidence"]["message_id"],
            "msg-profile-link"
        );
        assert_eq!(
            snapshot["person_profile_links"][0]["evidence"]["message_id"],
            "msg-person-link"
        );
    }

    #[tokio::test]
    async fn debug_snapshot_includes_groups_and_members() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        let person = Person {
            id: PersonId("person-group-debug".into()),
            name: Some("Group Member".into()),
            summary: None,
            comm_style: None,
            first_seen: 1000,
            last_seen: 1000,
        };
        store.add_person(&person).await.unwrap();
        store
            .add_group(&Group {
                id: GroupId("group-debug".into()),
                name: "Debug Group".into(),
                gateway_id: "discord".into(),
                external_id: "debug-channel".into(),
                context: GroupContext::Social,
                members: vec![person.id.clone()],
            })
            .await
            .unwrap();

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(snapshot["groups"][0]["id"], "group-debug");
        assert_eq!(snapshot["groups"][0]["name"], "Debug Group");
        assert_eq!(snapshot["groups"][0]["members"][0], "person-group-debug");
    }

    #[tokio::test]
    async fn debug_snapshot_includes_failed_events_without_payloads() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-failed-debug".into(),
                kind: "message".into(),
                payload: serde_json::json!({"content": "private message body"}),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: Some("event-failed-debug".into()),
                created_at: 1000,
                updated_at: 1000,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();
        store
            .mark_event_failed("event-failed-debug", 1001, Some("malformed payload"))
            .await
            .unwrap();

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(snapshot["failed_events"][0]["id"], "event-failed-debug");
        assert_eq!(snapshot["failed_events"][0]["kind"], "message");
        assert_eq!(
            snapshot["failed_events"][0]["last_error"],
            "malformed payload"
        );
        assert!(snapshot["failed_events"][0].get("payload").is_none());
    }

    #[tokio::test]
    async fn debug_snapshot_includes_review_jobs() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        store
            .start_action_run(&ActionRunRecord {
                action_id: "action-reviewed-debug".into(),
                kind: "respond".into(),
                task: "Respond before review".into(),
                conversation: Some(ConversationId("relay:local".into())),
                started_at: 1000,
                ended_at: Some(1001),
                status: "completed".into(),
                responded: true,
                attempts: 1,
            })
            .await
            .unwrap();
        store
            .mark_review_scheduled("action-reviewed-debug", "review-action-debug", 1002)
            .await
            .unwrap();

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(
            snapshot["review_jobs"][0]["source_action_id"],
            "action-reviewed-debug"
        );
        assert_eq!(
            snapshot["review_jobs"][0]["review_action_id"],
            "review-action-debug"
        );
        assert_eq!(snapshot["review_jobs"][0]["source_kind"], "respond");
    }

    #[tokio::test]
    async fn debug_snapshot_includes_action_traces() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        store
            .start_action_run(&ActionRunRecord {
                action_id: "action-debug".into(),
                kind: "respond".into(),
                task: "Respond to message".into(),
                conversation: None,
                started_at: 1000,
                ended_at: None,
                status: "running".into(),
                responded: false,
                attempts: 0,
            })
            .await
            .unwrap();
        store
            .append_action_turn(&ActionTurnRecord {
                action_id: "action-debug".into(),
                turn: 0,
                attempt: 1,
                prompt_hash: "hash-debug".into(),
                model: Some("model-debug".into()),
                finish: Some("tool_calls".into()),
                input_tokens: Some(10),
                output_tokens: Some(5),
                text_len: 12,
                reasoning_len: 0,
                tool_call_count: 1,
                created_at: 1001,
            })
            .await
            .unwrap();
        store
            .append_tool_call(&ToolCallRecord {
                action_id: "action-debug".into(),
                turn: 0,
                call_id: "call-debug".into(),
                name: "send_message".into(),
                args: serde_json::json!({"content": "hello"}),
                result: serde_json::json!({"status": "sent"}),
                success: true,
                started_at: 1002,
                ended_at: 1003,
            })
            .await
            .unwrap();
        store
            .append_action_message(&ActionMessageRecord {
                action_id: "action-debug".into(),
                role: "assistant".into(),
                conversation: Some(ConversationId("relay:local".into())),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-private".into()),
                sender_external_id: Some("sender-private".into()),
                reply_external_id: Some("reply-private".into()),
                content: Some("private reply body".into()),
                created_at: 1004,
            })
            .await
            .unwrap();
        store
            .append_outbound_delivery(&OutboundDeliveryRecord {
                action_id: "action-debug".into(),
                conversation: Some(ConversationId("relay:local".into())),
                gateway_id: "relay".into(),
                external_id: "reply-private".into(),
                status: "failed".into(),
                error: Some("delivery failed for reply-private".into()),
                attempted_at: 1005,
            })
            .await
            .unwrap();

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(snapshot["action_runs"][0]["task"], "[redacted]");
        assert_eq!(
            snapshot["action_traces"][0]["run"]["action_id"],
            "action-debug"
        );
        assert_eq!(snapshot["action_traces"][0]["run"]["task"], "[redacted]");
        assert_eq!(
            snapshot["action_traces"][0]["turns"][0]["prompt_hash"],
            "hash-debug"
        );
        assert_eq!(
            snapshot["action_traces"][0]["tool_calls"][0]["name"],
            "send_message"
        );
        assert_eq!(
            snapshot["action_traces"][0]["tool_calls"][0]["args"]["content"],
            "[redacted]"
        );
        assert_eq!(
            snapshot["action_traces"][0]["messages"][0]["content"],
            "[redacted]"
        );
        assert_eq!(
            snapshot["action_traces"][0]["messages"][0]["source_message_id"],
            "[redacted]"
        );
        assert_eq!(
            snapshot["action_traces"][0]["messages"][0]["reply_external_id"],
            "[redacted]"
        );
        assert_eq!(
            snapshot["action_traces"][0]["deliveries"][0]["external_id"],
            "[redacted]"
        );
        assert_eq!(
            snapshot["action_traces"][0]["deliveries"][0]["error"],
            "[redacted]"
        );
    }

    #[tokio::test]
    async fn debug_snapshot_includes_actor_metrics() {
        let store = actor::store::SqliteStore::open_in_memory(4).unwrap();
        let metrics = ActorMetrics::default();
        metrics.record_event_received();
        metrics.record_tool_call("send_message", true);

        let snapshot = debug_snapshot(&store, &metrics, 10).await.unwrap();

        assert_eq!(snapshot["metrics"]["events_received"], 1);
        assert_eq!(
            snapshot["metrics"]["tool_calls"]["send_message"]["success"],
            1
        );
    }
}
