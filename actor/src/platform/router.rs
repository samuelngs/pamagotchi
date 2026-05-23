use super::adapter::PlatformAdapter;
use super::content::MediaAttachment;
use std::collections::HashMap;
use std::sync::Arc;

pub struct PlatformRouter {
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
}

impl PlatformRouter {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn PlatformAdapter>) {
        self.adapters
            .insert(adapter.platform_id().to_string(), adapter);
    }

    pub fn get(&self, platform_id: &str) -> Option<&Arc<dyn PlatformAdapter>> {
        self.adapters.get(platform_id)
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

    pub async fn start_composing(
        &self,
        platform_id: &str,
        external_id: &str,
    ) -> anyhow::Result<()> {
        let adapter = self
            .adapters
            .get(platform_id)
            .ok_or_else(|| anyhow::anyhow!("unknown platform: {platform_id}"))?;
        adapter.start_composing(external_id).await
    }

    pub async fn stop_composing(
        &self,
        platform_id: &str,
        external_id: &str,
    ) -> anyhow::Result<()> {
        let adapter = self
            .adapters
            .get(platform_id)
            .ok_or_else(|| anyhow::anyhow!("unknown platform: {platform_id}"))?;
        adapter.stop_composing(external_id).await
    }
}
