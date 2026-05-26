use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayRuntimeEvent,
    GatewaySetupInstructions,
};
use async_trait::async_trait;
use protocol::{ConversationId, InboundMessage, MediaAttachment};
use relay::{RelayEvent, RelaySender};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct RelayAdapter {
    sender: RelaySender,
}

impl RelayAdapter {
    pub async fn connect(
        port: u16,
        channel: &str,
        inbound_tx: mpsc::Sender<InboundMessage>,
    ) -> anyhow::Result<Self> {
        let (sender, mut receiver) = relay::connect(port, channel).await?;

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    RelayEvent::Message { content } => {
                        let inbound = InboundMessage {
                            message_id: nanoid::nanoid!(),
                            gateway_id: "relay".into(),
                            external_id: "local".into(),
                            conversation: ConversationId("relay:local".into()),
                            group: None,
                            person: None,
                            content,
                            media: None,
                            timestamp: chrono::Utc::now().timestamp(),
                            metadata: serde_json::Value::Null,
                        };
                        if let Err(e) = inbound_tx.send(inbound).await {
                            warn!("failed to forward relay message: {e}");
                            break;
                        }
                    }
                    RelayEvent::ComposingStarted | RelayEvent::ComposingStopped => {
                        debug!("relay composing event (ignored on adapter side)");
                    }
                    RelayEvent::Subscribe { .. } => {}
                }
            }
            info!("relay listener stopped");
        });

        info!(port, "relay gateway adapter connected");
        Ok(Self { sender })
    }
}

#[async_trait]
impl GatewayAdapter for RelayAdapter {
    async fn connect(
        _id: String,
        _db_path: String,
        _inbound_tx: mpsc::Sender<InboundMessage>,
        _gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
    ) -> anyhow::Result<Self> {
        anyhow::bail!("relay adapter requires relay server port/channel connection")
    }

    fn gateway_id(&self) -> &str {
        "relay"
    }

    fn kind(&self) -> &str {
        "relay"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content_types: vec![],
            composing: true,
            read_receipts: false,
        }
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
        _media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()> {
        debug!(gateway = "relay", "sending message");
        self.sender
            .send(RelayEvent::Message {
                content: content.to_string(),
            })
            .await
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        debug!(gateway = "relay", "composing started");
        self.sender.send(RelayEvent::ComposingStarted).await
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        debug!(gateway = "relay", "composing stopped");
        self.sender.send(RelayEvent::ComposingStopped).await
    }
}
