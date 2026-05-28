use crate::content::GatewayCapabilities;
use async_trait::async_trait;
use media::MediaStore;
use protocol::{
    ConversationId, GatewayConnectionState, GatewaySetupInstructions, InboundMessage,
    MediaAttachment,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
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
    TypingUpdate {
        gateway_id: String,
        conversation: ConversationId,
        sender_external_id: String,
        typing: bool,
    },
    MessageEdited {
        gateway_id: String,
        conversation: ConversationId,
        message_id: String,
        content: String,
        edited_at: i64,
    },
    MessageDeleted {
        gateway_id: String,
        conversation: ConversationId,
        message_id: String,
        deleted_at: i64,
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
        media_store: Arc<MediaStore>,
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
        attachments: &[MediaAttachment],
    ) -> anyhow::Result<()>;
    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()>;
    async fn stop_composing(&self, external_id: &str) -> anyhow::Result<()>;
}
