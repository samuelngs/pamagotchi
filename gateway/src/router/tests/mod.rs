use super::*;
use crate::{GatewayCapabilities, GatewayContentCapabilities, GatewayRuntimeEvent};
use async_trait::async_trait;
use media::MediaStore;
use protocol::{GatewaySetupInstructions, InboundEnvelope};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;

struct StateAdapter {
    state: GatewayConnectionState,
    sends: AtomicUsize,
}

#[async_trait]
impl GatewayAdapter for StateAdapter {
    async fn connect(
        _id: String,
        _db_path: String,
        _vars: BTreeMap<String, Value>,
        _inbound_tx: mpsc::Sender<InboundEnvelope>,
        _gateway_event_tx: mpsc::Sender<GatewayRuntimeEvent>,
        _media_store: Arc<MediaStore>,
    ) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        anyhow::bail!("state adapter is only constructed directly")
    }

    fn kind(&self) -> &str {
        "state"
    }

    fn capabilities(&self) -> GatewayCapabilities {
        GatewayCapabilities {
            content: GatewayContentCapabilities::text_only(),
            composing: false,
            read_receipts: false,
        }
    }

    fn gateway_id(&self) -> &str {
        "relay"
    }

    fn connection_state(&self) -> GatewayConnectionState {
        self.state.clone()
    }

    fn setup_instructions(&self) -> Option<GatewaySetupInstructions> {
        None
    }

    async fn send_message(
        &self,
        _external_id: &str,
        _content: &str,
        _attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn start_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_composing(&self, _external_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn send_message_requires_connected_gateway() {
    let router = GatewayRouter::new();
    let adapter = Arc::new(StateAdapter {
        state: GatewayConnectionState::Disconnected,
        sends: AtomicUsize::new(0),
    });
    router.register(adapter.clone());

    let result = router.send_message("relay", "local", "hello", &[]).await;

    assert!(result.unwrap_err().to_string().contains("not connected"));
    assert_eq!(adapter.sends.load(Ordering::SeqCst), 0);
    assert!(!router.is_connected("relay"));
}
