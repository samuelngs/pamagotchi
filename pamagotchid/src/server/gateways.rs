use super::*;

pub(super) async fn attach_configured_gateways(
    gw_router: &Arc<GatewayRouter>,
    data_dir: &std::path::Path,
    inbound_tx: mpsc::Sender<InboundEnvelope>,
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

pub(super) async fn attach_configured_gateway(
    gw_router: &Arc<GatewayRouter>,
    data_dir: &std::path::Path,
    entry: &GatewayEntry,
    inbound_tx: mpsc::Sender<InboundEnvelope>,
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

pub(super) fn gateway_view(entry: &GatewayEntry, gw_router: &GatewayRouter) -> GatewayView {
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

pub(super) fn is_supported_gateway_kind(kind: &str) -> bool {
    supported_gateway_kinds().contains(&kind)
}

pub(super) fn supported_gateway_kinds() -> &'static [&'static str] {
    &["whatsapp", "discord"]
}

pub(super) fn gateway_kind_view(kind: &str) -> GatewayKindView {
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

pub(super) fn validate_gateway_vars(
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
