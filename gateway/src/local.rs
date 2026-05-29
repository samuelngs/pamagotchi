use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
use media::MediaStore;
use protocol::{InboundEnvelope, MediaAttachment, ServerEvent};
use relay::ApiServerHandle;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

pub struct LocalAdapter {
    handle: ApiServerHandle,
}

impl LocalAdapter {
    pub fn new(handle: ApiServerHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl GatewayAdapter for LocalAdapter {
    async fn connect(
        _id: String,
        _db_path: String,
        _vars: BTreeMap<String, Value>,
        _inbound_tx: mpsc::Sender<InboundEnvelope>,
        _gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        _media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self> {
        anyhow::bail!("local adapter requires an api server handle")
    }

    fn kind(&self) -> &str {
        "local"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities::text_only(),
            composing: true,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        "relay"
    }

    fn connection_state(&self) -> GatewayConnectionState {
        GatewayConnectionState::Connected
    }

    fn setup_instructions(&self) -> Option<GatewaySetupInstructions> {
        None
    }

    async fn send_message(
        &self,
        _external_id: &str,
        content: &str,
        _attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        debug!(gateway = "relay", "broadcasting local chat message");
        self.handle
            .broadcast(ServerEvent::ChatMessage {
                content: content.to_string(),
                is_self: false,
            })
            .await;
        Ok(())
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        debug!(gateway = "relay", "broadcasting local composing started");
        self.handle.broadcast(ServerEvent::ComposingStarted).await;
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        debug!(gateway = "relay", "broadcasting local composing stopped");
        self.handle.broadcast(ServerEvent::ComposingStopped).await;
        Ok(())
    }
}
