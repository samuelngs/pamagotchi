use super::*;

pub(super) fn spawn_api_request_handler(
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
            let gateway_id = protocol::GatewayId("relay".into());
            let inbound = InboundEnvelope {
                gateway_id: gateway_id.clone(),
                platform_message_id: format!("local-{}", now_millis()),
                channel: protocol::ChannelKey {
                    gateway_id: gateway_id.clone(),
                    external_id: "local".into(),
                    kind: protocol::ChannelKind::RelayRoom,
                    display_name: None,
                    space: None,
                    parent: None,
                    metadata: serde_json::json!({
                        "platform": "local_api",
                    }),
                },
                sender: Some(protocol::ObservedSender {
                    primary: protocol::ObservedIdentityKey {
                        gateway_id: gateway_id.clone(),
                        external_id: "local".into(),
                        kind: Some("relay_user".into()),
                        confidence: 1.0,
                        source: "primary_sender".into(),
                    },
                    aliases: vec![],
                    display_name: None,
                    metadata: serde_json::Value::Null,
                }),
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
