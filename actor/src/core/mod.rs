mod action;
mod event;
mod mind;
mod registry;
mod state;

pub use action::{
    ActionId, ActionKind, ActionRequest, ActionResult, ActionTiming, MindDecision,
    SupplementContext,
};
pub use event::{FiredIntent, InboundMessage, WakeEvent};
pub use mind::Mind;
pub use state::{SharedState, StateHandle};

use crate::personality::{GrowthConfig, PersonalityState};
use crate::store::{ActorConfig, Store};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info};

pub struct Actor {
    event_tx: mpsc::Sender<WakeEvent>,
    mind_handle: Option<JoinHandle<()>>,
    state_handle: Option<JoinHandle<()>>,
    state: StateHandle,
}

pub struct ActorBuilder {
    actor_config: ActorConfig,
    personality: PersonalityState,
    growth_config: GrowthConfig,
    store: Arc<dyn Store>,
    max_concurrency: usize,
    event_buffer: usize,
}

impl ActorBuilder {
    pub fn new(actor_config: ActorConfig, store: Arc<dyn Store>) -> Self {
        Self {
            actor_config,
            personality: PersonalityState::new(Default::default()),
            growth_config: GrowthConfig::default(),
            store,
            max_concurrency: 5,
            event_buffer: 256,
        }
    }

    pub fn with_personality(mut self, personality: PersonalityState) -> Self {
        self.personality = personality;
        self
    }

    pub fn with_growth_config(mut self, config: GrowthConfig) -> Self {
        self.growth_config = config;
        self
    }

    pub fn with_max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max;
        self
    }

    pub fn with_event_buffer(mut self, size: usize) -> Self {
        self.event_buffer = size;
        self
    }

    pub async fn build(self) -> anyhow::Result<Actor> {
        let (mut personality, mut growth_config) = (self.personality, self.growth_config);

        if let Some(snapshot) = self.store.load_latest_snapshot().await? {
            info!(saved_at = snapshot.saved_at, "restoring from snapshot");
            personality = snapshot.personality;
            growth_config = snapshot.config;
        }

        let shared = Arc::new(SharedState {
            personality: RwLock::new(personality),
            config: RwLock::new(growth_config),
            actor_config: RwLock::new(self.actor_config),
        });

        let (event_tx, event_rx) = mpsc::channel(self.event_buffer);
        let (delta_tx, delta_rx) = mpsc::channel(64);

        let state_handle = StateHandle::new(shared.clone(), delta_tx);

        let state_task = state::StateTask::new(shared.clone(), self.store.clone(), delta_rx);

        let mind = Mind::new(
            event_rx,
            event_tx.clone(),
            state_handle.clone(),
            self.store,
            self.max_concurrency,
        );

        let state_join = tokio::spawn(async move {
            state_task.run().await;
        });

        let mind_join = tokio::spawn(async move {
            mind.run().await;
        });

        info!("actor started");

        Ok(Actor {
            event_tx,
            mind_handle: Some(mind_join),
            state_handle: Some(state_join),
            state: state_handle,
        })
    }
}

impl Actor {
    pub fn builder(actor_config: ActorConfig, store: Arc<dyn Store>) -> ActorBuilder {
        ActorBuilder::new(actor_config, store)
    }

    pub async fn send_event(&self, event: WakeEvent) -> anyhow::Result<()> {
        self.event_tx
            .send(event)
            .await
            .map_err(|_| anyhow::anyhow!("actor event channel closed"))?;
        Ok(())
    }

    pub fn event_sender(&self) -> mpsc::Sender<WakeEvent> {
        self.event_tx.clone()
    }

    pub fn state(&self) -> &StateHandle {
        &self.state
    }

    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        info!("actor shutdown requested");

        if let Err(e) = self.event_tx.send(WakeEvent::Shutdown).await {
            error!(%e, "failed to send shutdown event");
        }

        if let Some(handle) = self.mind_handle.take() {
            handle.await.ok();
        }
        if let Some(handle) = self.state_handle.take() {
            handle.await.ok();
        }

        info!("actor shut down");
        Ok(())
    }
}
