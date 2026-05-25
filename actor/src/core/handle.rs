use crate::state::{ActorState, Delta, GrowthConfig};
use crate::store::{ActorSnapshot, Store};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct SharedState {
    pub actor: RwLock<ActorState>,
    pub config: RwLock<GrowthConfig>,
}

#[derive(Clone)]
pub struct StateHandle {
    pub shared: Arc<SharedState>,
    delta_tx: mpsc::Sender<Delta>,
}

impl StateHandle {
    pub fn new(shared: Arc<SharedState>, delta_tx: mpsc::Sender<Delta>) -> Self {
        Self { shared, delta_tx }
    }

    pub fn read_state(&self) -> std::sync::RwLockReadGuard<'_, ActorState> {
        self.shared.actor.read().unwrap()
    }

    pub fn read_config(&self) -> std::sync::RwLockReadGuard<'_, GrowthConfig> {
        self.shared.config.read().unwrap()
    }

    pub async fn send_delta(&self, delta: Delta) {
        self.delta_tx.send(delta).await.ok();
    }
}

pub struct StateTask {
    shared: Arc<SharedState>,
    store: Arc<dyn Store>,
    delta_rx: mpsc::Receiver<Delta>,
    dirty: bool,
}

impl StateTask {
    pub fn new(
        shared: Arc<SharedState>,
        store: Arc<dyn Store>,
        delta_rx: mpsc::Receiver<Delta>,
    ) -> Self {
        Self {
            shared,
            store,
            delta_rx,
            dirty: false,
        }
    }

    pub async fn run(mut self) {
        let save_interval = tokio::time::Duration::from_secs(300);
        loop {
            tokio::select! {
                maybe_delta = self.delta_rx.recv() => {
                    match maybe_delta {
                        Some(delta) => {
                            let mut batch = vec![delta];
                            while let Ok(d) = self.delta_rx.try_recv() {
                                batch.push(d);
                            }
                            self.apply_batch(batch);
                        }
                        None => {
                            self.save_if_dirty().await;
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(save_interval) => {
                    self.save_if_dirty().await;
                }
            }
        }
    }

    fn apply_batch(&mut self, batch: Vec<Delta>) {
        let config = self.shared.config.read().unwrap().clone();
        let mut state = self.shared.actor.write().unwrap();
        for delta in &batch {
            state.apply_delta(delta, &config);
        }
        drop(state);
        self.dirty = true;
        info!(count = batch.len(), "applied state deltas");
    }

    async fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let snapshot = {
            let state = self.shared.actor.read().unwrap().clone();
            let config = self.shared.config.read().unwrap().clone();
            ActorSnapshot {
                state,
                config,
                saved_at: now(),
            }
        };
        match self.store.save_snapshot(&snapshot).await {
            Ok(()) => {
                self.dirty = false;
                info!("saved actor snapshot");
            }
            Err(e) => {
                warn!(%e, "failed to save actor snapshot");
            }
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
