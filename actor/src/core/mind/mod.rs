mod evaluate;
mod event;
mod respond;
mod spawn;

use super::action::ActionId;
use super::event::WakeEvent;
use super::registry::ActionRegistry;
use super::state::StateHandle;
use inference::InferenceRouter;
use gateway::GatewayRouter;
use crate::store::Store;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

pub struct Mind {
    pub(super) event_rx: mpsc::Receiver<WakeEvent>,
    pub(super) event_tx: mpsc::Sender<WakeEvent>,
    pub(super) registry: ActionRegistry,
    pub(super) state: StateHandle,
    pub(super) store: Arc<dyn Store>,
    pub(super) router: Arc<InferenceRouter>,
    pub(super) gateway: Arc<GatewayRouter>,
    pub(super) max_turns: usize,
}

impl Mind {
    pub fn new(
        event_rx: mpsc::Receiver<WakeEvent>,
        event_tx: mpsc::Sender<WakeEvent>,
        state: StateHandle,
        store: Arc<dyn Store>,
        router: Arc<InferenceRouter>,
        gateway: Arc<GatewayRouter>,
        max_concurrency: usize,
        max_turns: usize,
    ) -> Self {
        Self {
            event_rx,
            event_tx,
            registry: ActionRegistry::new(max_concurrency),
            state,
            store,
            router,
            gateway,
            max_turns,
        }
    }

    pub async fn run(mut self) {
        info!("mind started");
        loop {
            match self.event_rx.recv().await {
                Some(WakeEvent::Shutdown) => {
                    self.shutdown().await;
                    break;
                }
                Some(WakeEvent::ActionCompleted { action_id, result }) => {
                    self.registry.mark_completed(&action_id);
                    self.handle_action_completed(&action_id, &result).await;
                    let event = WakeEvent::ActionCompleted { action_id, result };
                    let verdict = self.evaluate(&event).await;
                    let decision = self.build_decision(verdict, &event);
                    self.execute_decision(decision).await;
                    self.registry.gc();
                }
                Some(event) => {
                    let verdict = self.evaluate(&event).await;
                    let decision = self.build_decision(verdict, &event);
                    self.execute_decision(decision).await;
                    self.registry.gc();
                }
                None => {
                    info!("event channel closed, shutting down");
                    self.shutdown().await;
                    break;
                }
            }
        }
        info!("mind stopped");
    }

    async fn shutdown(&mut self) {
        info!("mind shutting down, cancelling all actions");
        let running: Vec<ActionId> = self
            .registry
            .running_actions()
            .iter()
            .map(|a| a.id.clone())
            .collect();
        for id in &running {
            self.registry.cancel(id);
        }
    }
}
