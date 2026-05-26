use crate::content::GatewayCapabilities;
use async_trait::async_trait;
use protocol::{GatewayConnectionState, GatewaySetupInstructions, InboundMessage, MediaAttachment};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GatewayRuntimeEvent {
    ConnectionStateChanged {
        gateway_id: String,
        state: GatewayConnectionState,
    },
    SetupInstructionsChanged {
        gateway_id: String,
        setup: Option<GatewaySetupInstructions>,
    },
}

#[async_trait]
pub trait GatewayAdapter: Send + Sync {
    async fn connect(
        id: String,
        db_path: String,
        vars: BTreeMap<String, Value>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
    ) -> anyhow::Result<Self>
    where
        Self: Sized;

    fn kind(&self) -> &str;
    fn capabilities(&self) -> GatewayCapabilities;
    fn gateway_id(&self) -> &str;
    fn connection_state(&self) -> GatewayConnectionState;
    fn setup_instructions(&self) -> Option<GatewaySetupInstructions>;
    async fn send_message(
        &self,
        external_id: &str,
        content: &str,
        media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()>;
    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()>;
    async fn stop_composing(&self, external_id: &str) -> anyhow::Result<()>;
}
