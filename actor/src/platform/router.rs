use super::adapter::PlatformAdapter;
use super::content::MediaAttachment;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{info, warn};

struct ComposingEntry {
    count: usize,
    acquired_at: Instant,
    platform_id: String,
    external_id: String,
}

pub struct PlatformRouter {
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
    composing: Arc<Mutex<HashMap<String, ComposingEntry>>>,
}

const COMPOSING_TIMEOUT_SECS: u64 = 120;

impl PlatformRouter {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
            composing: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn PlatformAdapter>) {
        self.adapters
            .insert(adapter.platform_id().to_string(), adapter);
    }

    pub fn get(&self, platform_id: &str) -> Option<&Arc<dyn PlatformAdapter>> {
        self.adapters.get(platform_id)
    }

    pub fn start_composing_sweep(&self) {
        let composing = self.composing.clone();
        let adapters: HashMap<String, Arc<dyn PlatformAdapter>> = self.adapters.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let expired: Vec<(String, String)> = {
                    let mut map = composing.lock().await;
                    let mut to_remove = vec![];
                    for (key, entry) in map.iter() {
                        if entry.acquired_at.elapsed().as_secs() > COMPOSING_TIMEOUT_SECS {
                            to_remove.push((key.clone(), entry.platform_id.clone(), entry.external_id.clone()));
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
                    warn!(platform = %pid, external_id = %eid, "composing timeout, force-releasing");
                    if let Some(adapter) = adapters.get(&pid) {
                        adapter.stop_composing(&eid).await.ok();
                    }
                }
            }
        });
    }

    pub async fn acquire_composing(
        &self,
        platform_id: &str,
        external_id: &str,
    ) {
        let key = format!("{platform_id}:{external_id}");
        let should_start = {
            let mut map = self.composing.lock().await;
            let entry = map.entry(key).or_insert(ComposingEntry {
                count: 0,
                acquired_at: Instant::now(),
                platform_id: platform_id.to_string(),
                external_id: external_id.to_string(),
            });
            entry.count += 1;
            entry.count == 1
        };
        if should_start {
            if let Some(adapter) = self.adapters.get(platform_id) {
                if let Err(e) = adapter.start_composing(external_id).await {
                    warn!(%e, platform = %platform_id, "acquire_composing: start_composing failed");
                } else {
                    info!(platform = %platform_id, external_id = %external_id, "composing started");
                }
            }
        }
    }

    pub async fn release_composing(
        &self,
        platform_id: &str,
        external_id: &str,
    ) {
        let key = format!("{platform_id}:{external_id}");
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
            if let Some(adapter) = self.adapters.get(platform_id) {
                if let Err(e) = adapter.stop_composing(external_id).await {
                    warn!(%e, platform = %platform_id, "release_composing: stop_composing failed");
                } else {
                    info!(platform = %platform_id, external_id = %external_id, "composing stopped");
                }
            }
        }
    }

    pub async fn send_message(
        &self,
        platform_id: &str,
        external_id: &str,
        content: &str,
        media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()> {
        let adapter = self
            .adapters
            .get(platform_id)
            .ok_or_else(|| anyhow::anyhow!("unknown platform: {platform_id}"))?;
        adapter.send_message(external_id, content, media).await
    }
}
