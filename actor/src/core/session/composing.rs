use super::*;

pub(super) fn session_cancellation_token(
    ctx: &SessionContext,
) -> crate::core::action::CancellationToken {
    ctx.progress
        .read()
        .map(|progress| progress.cancellation_token())
        .unwrap_or_else(|_| crate::core::action::RunningState::new().cancellation_token())
}

pub(super) struct ComposingGuard {
    gateway: Arc<GatewayRouter>,
    target: Option<(String, String)>,
}

impl ComposingGuard {
    pub(super) fn new(
        gateway: Arc<GatewayRouter>,
        gateway_id: String,
        external_id: String,
    ) -> Self {
        Self {
            gateway,
            target: Some((gateway_id, external_id)),
        }
    }

    pub(super) async fn release(&mut self) {
        if let Some((gateway_id, external_id)) = self.target.take() {
            self.gateway
                .release_composing(&gateway_id, &external_id)
                .await;
        }
    }

    pub(super) fn disarm(&mut self) {
        self.target = None;
    }
}

impl Drop for ComposingGuard {
    fn drop(&mut self) {
        let Some((gateway_id, external_id)) = self.target.take() else {
            return;
        };
        let gateway = self.gateway.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                gateway.release_composing(&gateway_id, &external_id).await;
            });
        }
    }
}
