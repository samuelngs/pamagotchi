use super::*;
use crate::tui::focus::FocusId;

impl App {
    pub async fn connect(&mut self) -> anyhow::Result<()> {
        let api = ApiClient::connect(self.port).await?;
        api.send(ClientRequest::Subscribe {
            topics: vec![SubscriptionTopic::Chat, SubscriptionTopic::Gateways],
        })
        .await?;
        self.api = Some(api);
        Ok(())
    }

    pub fn poll_api(&mut self) {
        let api = match &mut self.api {
            Some(api) => api,
            None => return,
        };
        while let Some(event) = api.try_recv() {
            match event {
                ServerEvent::ChatMessage { content, is_self } => {
                    self.messages.push(ChatMessage { content, is_self });
                    self.messages_scroll = 0;
                    self.composing = false;
                }
                ServerEvent::ComposingStarted => {
                    self.composing = true;
                }
                ServerEvent::ComposingStopped => {
                    self.composing = false;
                }
                ServerEvent::GatewayList { gateways, .. } => {
                    self.gateways = gateways;
                    if self.screen == Screen::Gateways
                        && self.focus.is(FocusId::GatewayList)
                        && self.gateways.is_empty()
                    {
                        self.focus.set(FocusId::GatewayBack);
                    }
                    if self.gateways_selection >= self.gateways.len() {
                        self.gateways_selection = self.gateways.len().saturating_sub(1);
                    }
                }
                ServerEvent::AvailableGatewayList { gateways, .. } => {
                    self.available_gateways = gateways;
                    if self.add_selection >= self.available_gateways.len() {
                        self.add_selection = self.available_gateways.len().saturating_sub(1);
                    }
                }
                ServerEvent::GatewayAdded { gateway } => {
                    self.gateways.push(gateway);
                }
                ServerEvent::GatewayRemoved { id } => {
                    self.gateways.retain(|g| g.id != id);
                    if self.selected_gateway_id.as_ref() == Some(&id) {
                        self.selected_gateway_id = None;
                    }
                }
                ServerEvent::GatewayUpdated { gateway } => {
                    if let Some(existing) = self.gateways.iter_mut().find(|g| g.id == gateway.id) {
                        *existing = gateway;
                    }
                }
                ServerEvent::GatewayConnectionStateChanged { id, state } => {
                    if let Some(gw) = self.gateways.iter_mut().find(|g| g.id == id) {
                        gw.connection_state = state;
                    }
                }
                ServerEvent::GatewaySetupInstructionsChanged { id, setup } => {
                    if let Some(gw) = self.gateways.iter_mut().find(|g| g.id == id) {
                        gw.setup_instructions = setup;
                    }
                }
                ServerEvent::DebugSnapshot {
                    request_id,
                    snapshot,
                } => {
                    if self.debug_request_id.as_deref() == Some(request_id.as_str()) {
                        self.debug_snapshot = crate::debug_view::format_snapshot(&snapshot);
                        self.debug_request_id = None;
                        self.debug_scroll = 0;
                    }
                }
                ServerEvent::RequestError {
                    request_id: Some(request_id),
                    message,
                } if self.debug_request_id.as_deref() == Some(request_id.as_str()) => {
                    self.debug_snapshot = format!("Debug snapshot failed: {message}");
                    self.debug_request_id = None;
                    self.debug_scroll = 0;
                }
                ServerEvent::MediaAssetCreated { .. }
                | ServerEvent::RequestOk { .. }
                | ServerEvent::RequestError { .. } => {}
            }
        }
    }

    pub async fn submit_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }

        self.messages.push(ChatMessage {
            content: text.clone(),
            is_self: true,
        });
        self.messages_scroll = 0;

        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::SendChatMessage { content: text })
                .await;
        }

        self.input.clear();
        self.cursor = 0;
        self.input_scroll = 0;
    }

    pub async fn request_gateway_list(&mut self) {
        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::ListGateways {
                    request_id: request_id("list"),
                })
                .await;
            let _ = api
                .send(ClientRequest::ListAvailableGateways {
                    request_id: request_id("available"),
                })
                .await;
        }
    }

    pub async fn request_debug_snapshot(&mut self) {
        let request_id = request_id("debug");
        self.debug_request_id = Some(request_id.clone());
        self.debug_snapshot = "Loading debug snapshot...".into();
        self.debug_scroll = 0;
        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::GetDebugSnapshot {
                    request_id,
                    limit: Some(25),
                })
                .await;
        }
    }
}
