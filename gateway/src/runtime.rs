use crate::{GatewayConnectionState, GatewayRuntimeEvent, GatewaySetupInstructions};
use std::sync::RwLock;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct GatewayRuntime {
    state: RwLock<GatewayConnectionState>,
    setup: RwLock<Option<GatewaySetupInstructions>>,
    event_tx: mpsc::Sender<GatewayRuntimeEvent>,
}

impl GatewayRuntime {
    pub fn new(event_tx: mpsc::Sender<GatewayRuntimeEvent>) -> Self {
        Self {
            state: RwLock::new(GatewayConnectionState::Connecting),
            setup: RwLock::new(None),
            event_tx,
        }
    }

    pub async fn emit_state(&self, gateway_id: &str, state: GatewayConnectionState) {
        *self.state.write().unwrap() = state.clone();
        let _ = self
            .event_tx
            .send(GatewayRuntimeEvent::ConnectionStateChanged {
                gateway_id: gateway_id.to_string(),
                state,
            })
            .await;
    }

    pub async fn emit_setup(&self, gateway_id: &str, setup: Option<GatewaySetupInstructions>) {
        *self.setup.write().unwrap() = setup.clone();
        let _ = self
            .event_tx
            .send(GatewayRuntimeEvent::SetupInstructionsChanged {
                gateway_id: gateway_id.to_string(),
                setup,
            })
            .await;
    }

    pub fn connection_state(&self) -> GatewayConnectionState {
        self.state.read().unwrap().clone()
    }

    pub fn setup_instructions(&self) -> Option<GatewaySetupInstructions> {
        self.setup.read().unwrap().clone()
    }
}
