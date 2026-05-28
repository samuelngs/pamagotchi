use crate::adapter::GatewayAdapter;
use protocol::{GatewayConnectionState, MediaAttachment};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{info, warn};

struct ComposingEntry {
    count: usize,
    acquired_at: Instant,
    gateway_id: String,
    external_id: String,
}

pub struct GatewayRouter {
    adapters: Arc<RwLock<HashMap<String, Arc<dyn GatewayAdapter>>>>,
    composing: Arc<Mutex<HashMap<String, ComposingEntry>>>,
}

const COMPOSING_TIMEOUT_SECS: u64 = 120;

impl GatewayRouter {
    pub fn new() -> Self {
        Self {
            adapters: Arc::new(RwLock::new(HashMap::new())),
            composing: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register(&self, adapter: Arc<dyn GatewayAdapter>) {
        self.adapters
            .write()
            .unwrap()
            .insert(adapter.gateway_id().to_string(), adapter);
    }

    pub fn unregister(&self, gateway_id: &str) -> Option<Arc<dyn GatewayAdapter>> {
        self.adapters.write().unwrap().remove(gateway_id)
    }

    pub fn get(&self, gateway_id: &str) -> Option<Arc<dyn GatewayAdapter>> {
        self.adapters.read().unwrap().get(gateway_id).cloned()
    }

    pub fn connection_state(&self, gateway_id: &str) -> Option<GatewayConnectionState> {
        self.get(gateway_id)
            .map(|adapter| adapter.connection_state())
    }

    pub fn is_connected(&self, gateway_id: &str) -> bool {
        matches!(
            self.connection_state(gateway_id),
            Some(GatewayConnectionState::Connected)
        )
    }

    pub fn list(&self) -> Vec<Arc<dyn GatewayAdapter>> {
        self.adapters.read().unwrap().values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.adapters.read().unwrap().len()
    }

    pub fn start_composing_sweep(&self) {
        let composing = self.composing.clone();
        let adapters = self.adapters.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let expired: Vec<(String, String)> = {
                    let mut map = composing.lock().await;
                    let mut to_remove = vec![];
                    for (key, entry) in map.iter() {
                        if entry.acquired_at.elapsed().as_secs() > COMPOSING_TIMEOUT_SECS {
                            to_remove.push((
                                key.clone(),
                                entry.gateway_id.clone(),
                                entry.external_id.clone(),
                            ));
                        }
                    }
                    let mut pairs = vec![];
                    for (key, pid, eid) in to_remove {
                        map.remove(&key);
                        pairs.push((pid, eid));
                    }
                    pairs
                };
                for (pid, eid) in expired {
                    warn!(gateway = %pid, external_id = %eid, "composing timeout, force-releasing");
                    let adapter = adapters.read().unwrap().get(&pid).cloned();
                    if let Some(adapter) = adapter {
                        adapter.stop_composing(&eid).await.ok();
                    }
                }
            }
        });
    }

    pub async fn acquire_composing(&self, gateway_id: &str, external_id: &str) {
        let key = format!("{gateway_id}:{external_id}");
        let should_start = {
            let mut map = self.composing.lock().await;
            let entry = map.entry(key).or_insert(ComposingEntry {
                count: 0,
                acquired_at: Instant::now(),
                gateway_id: gateway_id.to_string(),
                external_id: external_id.to_string(),
            });
            entry.count += 1;
            entry.count == 1
        };
        if should_start {
            if let Some(adapter) = self.get(gateway_id) {
                if let Err(e) = adapter.start_composing(external_id).await {
                    warn!(%e, gateway = %gateway_id, "acquire_composing: start_composing failed");
                } else {
                    info!(gateway = %gateway_id, external_id = %external_id, "composing started");
                }
            }
        }
    }

    pub async fn release_composing(&self, gateway_id: &str, external_id: &str) {
        let key = format!("{gateway_id}:{external_id}");
        let should_stop = {
            let mut map = self.composing.lock().await;
            if let Some(entry) = map.get_mut(&key) {
                entry.count = entry.count.saturating_sub(1);
                if entry.count == 0 {
                    map.remove(&key);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if should_stop {
            if let Some(adapter) = self.get(gateway_id) {
                if let Err(e) = adapter.stop_composing(external_id).await {
                    warn!(%e, gateway = %gateway_id, "release_composing: stop_composing failed");
                } else {
                    info!(gateway = %gateway_id, external_id = %external_id, "composing stopped");
                }
            }
        }
    }

    pub async fn composing_count(&self, gateway_id: &str, external_id: &str) -> usize {
        let key = format!("{gateway_id}:{external_id}");
        self.composing
            .lock()
            .await
            .get(&key)
            .map_or(0, |entry| entry.count)
    }

    pub async fn send_message(
        &self,
        gateway_id: &str,
        external_id: &str,
        content: &str,
        attachments: &[MediaAttachment],
    ) -> anyhow::Result<()> {
        let adapter = self
            .get(gateway_id)
            .ok_or_else(|| anyhow::anyhow!("unknown gateway: {gateway_id}"))?;
        let state = adapter.connection_state();
        if !matches!(state, GatewayConnectionState::Connected) {
            anyhow::bail!("gateway {gateway_id} is not connected: {state:?}");
        }
        adapter
            .send_message(external_id, content, attachments)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GatewayCapabilities, GatewayContentCapabilities, GatewayRuntimeEvent};
    use async_trait::async_trait;
    use media::MediaStore;
    use protocol::{GatewaySetupInstructions, InboundMessage};
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
            _inbound_tx: mpsc::Sender<InboundMessage>,
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
}
