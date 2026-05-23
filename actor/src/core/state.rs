use crate::personality::{GrowthConfig, PersonalityDelta, PersonalityState};
use crate::store::{ActorConfig, ActorSnapshot, Store};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct SharedState {
    pub personality: RwLock<PersonalityState>,
    pub config: RwLock<GrowthConfig>,
    pub actor_config: RwLock<ActorConfig>,
}

#[derive(Clone)]
pub struct StateHandle {
    pub shared: Arc<SharedState>,
    delta_tx: mpsc::Sender<PersonalityDelta>,
}

impl StateHandle {
    pub fn new(shared: Arc<SharedState>, delta_tx: mpsc::Sender<PersonalityDelta>) -> Self {
        Self { shared, delta_tx }
    }

    pub fn read_personality(&self) -> std::sync::RwLockReadGuard<'_, PersonalityState> {
        self.shared.personality.read().unwrap()
    }

    pub fn read_config(&self) -> std::sync::RwLockReadGuard<'_, GrowthConfig> {
        self.shared.config.read().unwrap()
    }

    pub fn read_actor_config(&self) -> std::sync::RwLockReadGuard<'_, ActorConfig> {
        self.shared.actor_config.read().unwrap()
    }

    pub async fn send_delta(&self, delta: PersonalityDelta) {
        self.delta_tx.send(delta).await.ok();
    }
}

pub struct StateTask {
    shared: Arc<SharedState>,
    store: Arc<dyn Store>,
    delta_rx: mpsc::Receiver<PersonalityDelta>,
    dirty: bool,
}

impl StateTask {
    pub fn new(
        shared: Arc<SharedState>,
        store: Arc<dyn Store>,
        delta_rx: mpsc::Receiver<PersonalityDelta>,
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

    fn apply_batch(&mut self, batch: Vec<PersonalityDelta>) {
        let config = self.shared.config.read().unwrap().clone();
        let mut personality = self.shared.personality.write().unwrap();
        for delta in &batch {
            personality.apply_delta(delta, &config);
        }
        drop(personality);
        self.dirty = true;
        info!(count = batch.len(), "applied personality deltas");
    }

    async fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let snapshot = {
            let personality = self.shared.personality.read().unwrap().clone();
            let config = self.shared.config.read().unwrap().clone();
            let actor_config = self.shared.actor_config.read().unwrap().clone();
            ActorSnapshot {
                actor: actor_config,
                personality,
                config,
                saved_at: now(),
            }
        };
        match self.store.save_snapshot(&snapshot).await {
            Ok(()) => {
                self.dirty = false;
                info!("saved personality snapshot");
            }
            Err(e) => {
                warn!(%e, "failed to save personality snapshot");
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
