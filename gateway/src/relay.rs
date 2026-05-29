use crate::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayRuntimeEvent, GatewaySetupInstructions,
};
use async_trait::async_trait;
use media::MediaStore;
use protocol::{
    ChannelKey, ChannelKind, GatewayId, InboundEnvelope, MediaAttachment, ObservedIdentityKey,
    ObservedSender,
};
use relay::{RelayEvent, RelaySender};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct RelayAdapter {
    sender: RelaySender,
}

impl RelayAdapter {
    pub async fn connect(
        port: u16,
        channel: &str,
        inbound_tx: mpsc::Sender<InboundEnvelope>,
    ) -> anyhow::Result<Self> {
        let (sender, mut receiver) = relay::connect(port, channel).await?;

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    RelayEvent::Message { content } => {
                        let gateway = GatewayId("relay".into());
                        let inbound = InboundEnvelope {
                            gateway_id: gateway.clone(),
                            platform_message_id: nanoid::nanoid!(),
                            channel: relay_channel_key(&gateway, "local"),
                            sender: Some(ObservedSender {
                                primary: relay_identity_key(&gateway, "local"),
                                aliases: vec![],
                                display_name: None,
                                metadata: serde_json::Value::Null,
                            }),
                            content,
                            attachments: Vec::new(),
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
        _vars: BTreeMap<String, Value>,
        _inbound_tx: mpsc::Sender<InboundEnvelope>,
        _gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        _media_store: Arc<MediaStore>,
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
            content: GatewayContentCapabilities::text_only(),
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
        _attachments: &[MediaAttachment],
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

fn relay_channel_key(gateway_id: &GatewayId, room: &str) -> ChannelKey {
    ChannelKey {
        gateway_id: gateway_id.clone(),
        external_id: room.to_string(),
        kind: ChannelKind::RelayRoom,
        display_name: None,
        space: None,
        parent: None,
        metadata: serde_json::json!({
            "platform": "relay",
        }),
    }
}

fn relay_identity_key(gateway_id: &GatewayId, user: &str) -> ObservedIdentityKey {
    ObservedIdentityKey {
        gateway_id: gateway_id.clone(),
        external_id: user.to_string(),
        kind: Some("relay_user".into()),
        confidence: 1.0,
        source: "primary_sender".into(),
    }
}
