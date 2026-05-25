use super::mind::Mind;
use super::handle::{self, SharedState, StateHandle};
use super::event::WakeEvent;
use inference::InferenceRouter;
use crate::state::{ActorState, GrowthConfig};
use gateway::GatewayRouter;
use crate::store::Store;
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
    actor_state: ActorState,
    growth_config: GrowthConfig,
    store: Arc<dyn Store>,
    router: Arc<InferenceRouter>,
    gateway: Arc<GatewayRouter>,
    max_concurrency: usize,
    max_turns: usize,
    event_buffer: usize,
    event_channel: Option<(mpsc::Sender<WakeEvent>, mpsc::Receiver<WakeEvent>)>,
}

impl ActorBuilder {
    pub fn new(store: Arc<dyn Store>, router: Arc<InferenceRouter>) -> Self {
        Self {
            actor_state: ActorState::new(Default::default()),
            growth_config: GrowthConfig::default(),
            store,
            router,
            gateway: Arc::new(GatewayRouter::new()),
            max_concurrency: 5,
            max_turns: 5,
            event_buffer: 256,
            event_channel: None,
        }
    }

    pub fn with_state(mut self, state: ActorState) -> Self {
        self.actor_state = state;
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

    pub fn with_max_turns(mut self, max: usize) -> Self {
        self.max_turns = max;
        self
    }

    pub fn with_gateway(mut self, gateway: GatewayRouter) -> Self {
        self.gateway = Arc::new(gateway);
        self
    }

    pub fn with_event_buffer(mut self, size: usize) -> Self {
        self.event_buffer = size;
        self
    }

    pub fn with_event_channel(
        mut self,
        tx: mpsc::Sender<WakeEvent>,
        rx: mpsc::Receiver<WakeEvent>,
    ) -> Self {
        self.event_channel = Some((tx, rx));
        self
    }

    pub async fn build(self) -> anyhow::Result<Actor> {
        let (mut actor_state, mut growth_config) = (self.actor_state, self.growth_config);

        if let Some(snapshot) = self.store.load_latest_snapshot().await? {
            info!(saved_at = snapshot.saved_at, "restoring from snapshot");
            actor_state = snapshot.state;
            growth_config = snapshot.config;
        }

        let shared = Arc::new(SharedState {
            actor: RwLock::new(actor_state),
            config: RwLock::new(growth_config),
        });

        let (event_tx, event_rx) = self
            .event_channel
            .unwrap_or_else(|| mpsc::channel(self.event_buffer));
        let (delta_tx, delta_rx) = mpsc::channel(64);

        let state_handle = StateHandle::new(shared.clone(), delta_tx);

        let state_task = handle::StateTask::new(shared.clone(), self.store.clone(), delta_rx);

        let mind = Mind::new(
            event_rx,
            event_tx.clone(),
            state_handle.clone(),
            self.store,
            self.router,
            self.gateway,
            self.max_concurrency,
            self.max_turns,
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
    pub fn builder(store: Arc<dyn Store>, router: Arc<InferenceRouter>) -> ActorBuilder {
        ActorBuilder::new(store, router)
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

        drop(self.state);

        if let Some(handle) = self.state_handle.take() {
            handle.await.ok();
        }

        info!("actor shut down");
        Ok(())
    }
}
