use async_trait::async_trait;
use gateway::{
    GatewayAdapter, GatewayCapabilities, GatewayConnectionState, GatewayContentCapabilities,
    GatewayRuntimeEvent,
};
use media::MediaStore;
use protocol::{GatewaySetupInstructions, InboundMessage, MediaAttachment};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc};

#[derive(Clone, Debug)]
pub struct CapturedOutbound {
    pub gateway_id: String,
    pub external_id: String,
    pub content: String,
    pub attachment_count: usize,
}

#[derive(Clone, Default)]
pub struct CaptureSink {
    inner: Arc<CaptureSinkInner>,
}

#[derive(Default)]
struct CaptureSinkInner {
    messages: Mutex<Vec<CapturedOutbound>>,
    notify: Notify,
}

impl CaptureSink {
    pub fn messages(&self) -> Vec<CapturedOutbound> {
        self.inner.messages.lock().unwrap().clone()
    }

    pub async fn wait_for_change(&self) {
        self.inner.notify.notified().await;
    }

    fn push(&self, message: CapturedOutbound) {
        self.inner.messages.lock().unwrap().push(message);
        self.inner.notify.notify_waiters();
    }
}

pub struct RecordingGateway {
    gateway_id: String,
    sink: CaptureSink,
}

impl RecordingGateway {
    pub fn new(gateway_id: impl Into<String>, sink: CaptureSink) -> Self {
        Self {
            gateway_id: gateway_id.into(),
            sink,
        }
    }
}

#[async_trait]
impl GatewayAdapter for RecordingGateway {
    async fn connect(
        _id: String,
        _db_path: String,
        _vars: BTreeMap<String, Value>,
        _inbound_tx: mpsc::Sender<InboundMessage>,
        _gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        _media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        anyhow::bail!("recording gateway is only constructed directly")
    }

    fn kind(&self) -> &str {
        "behaviour-spec-recording"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities::text_only(),
            composing: true,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        &self.gateway_id
    }

    fn connection_state(&self) -> GatewayConnectionState {
        GatewayConnectionState::Connected
    }

    fn setup_instructions(&self) -> Option<GatewaySetupInstructions> {
        None
    }

    async fn send_message(
        &self,
        external_id: &str,
        content: &str,
        attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        self.sink.push(CapturedOutbound {
            gateway_id: self.gateway_id.clone(),
            external_id: external_id.to_string(),
            content: content.to_string(),
            attachment_count: attachments.len(),
        });
        Ok(())
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
