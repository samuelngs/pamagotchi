use crate::content::GatewayCapabilities;
use protocol::MediaAttachment;
use async_trait::async_trait;

#[async_trait]
pub trait GatewayAdapter: Send + Sync {
    fn gateway_id(&self) -> &str;
    fn capabilities(&self) -> GatewayCapabilities;
    async fn send_message(
        &self,
        external_id: &str,
        content: &str,
        media: Option<&MediaAttachment>,
    ) -> anyhow::Result<()>;
    async fn start_composing(&self, external_id: &str) -> anyhow::Result<()>;
    async fn stop_composing(&self, external_id: &str) -> anyhow::Result<()>;
}
