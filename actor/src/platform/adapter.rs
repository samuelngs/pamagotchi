use super::content::{MediaAttachment, PlatformCapabilities};
use async_trait::async_trait;

#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    fn platform_id(&self) -> &str;
    fn capabilities(&self) -> PlatformCapabilities;
    async fn send_message(
        &self,
        external_id: &str,
        content: &str,
        media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()>;
    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()>;
    async fn stop_composing(&self, external_id: &str) -> anyhow::Result<()>;
}
