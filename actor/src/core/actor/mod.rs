use super::event::WakeEvent;
use super::handle::{self, SharedState, StateHandle};
use super::metrics::{ActorMetrics, ActorMetricsSnapshot};
use super::mind::Mind;
use super::scheduler::spawn_scheduler;
use crate::state::{ActorState, Authority, Delta, GrowthConfig};
use crate::store::Store;
use gateway::GatewayRouter;
use inference::InferenceRouter;
use media::MediaStore;
use protocol::PersonId;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

pub struct Actor {
    event_tx: mpsc::Sender<WakeEvent>,
    mind_handle: Option<JoinHandle<()>>,
    state_handle: Option<JoinHandle<()>>,
    scheduler_handle: Option<JoinHandle<()>>,
    state: StateHandle,
    metrics: Arc<ActorMetrics>,
}

pub struct ActorBuilder {
    actor_state: ActorState,
    growth_config: GrowthConfig,
    store: Arc<dyn Store>,
    media_store: Option<Arc<MediaStore>>,
    router: Arc<InferenceRouter>,
    gateway: Arc<GatewayRouter>,
    max_concurrency: usize,
    max_turns: usize,
    max_action_attempts: usize,
    escalate_after: usize,
    event_buffer: usize,
    event_channel: Option<(mpsc::Sender<WakeEvent>, mpsc::Receiver<WakeEvent>)>,
    metrics: Arc<ActorMetrics>,
}

impl ActorBuilder {
    pub fn new(store: Arc<dyn Store>, router: Arc<InferenceRouter>) -> Self {
        Self {
            actor_state: ActorState::new(Default::default()),
            growth_config: GrowthConfig::default(),
            store,
            media_store: None,
            router,
            gateway: Arc::new(GatewayRouter::new()),
            max_concurrency: 5,
            max_turns: 5,
            max_action_attempts: 3,
            escalate_after: 1,
            event_buffer: 256,
            event_channel: None,
            metrics: Arc::new(ActorMetrics::default()),
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

    pub fn with_retry(mut self, max_attempts: usize, escalate_after: usize) -> Self {
        self.max_action_attempts = max_attempts;
        self.escalate_after = escalate_after;
        self
    }

    pub fn with_gateway(mut self, gateway: Arc<GatewayRouter>) -> Self {
        self.gateway = gateway;
        self
    }

    pub fn with_media_store(mut self, media_store: Arc<MediaStore>) -> Self {
        self.media_store = Some(media_store);
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

    pub fn with_metrics(mut self, metrics: Arc<ActorMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    pub async fn build(self) -> anyhow::Result<Actor> {
        let (mut actor_state, mut growth_config) = (self.actor_state, self.growth_config);
        let mut last_state_journal_id = None;

        if let Some(snapshot) = self.store.load_latest_snapshot().await? {
            info!(saved_at = snapshot.saved_at, "restoring from snapshot");
            last_state_journal_id = snapshot.last_state_journal_id;
            actor_state = snapshot.state;
            growth_config = snapshot.config;
        }
        last_state_journal_id = replay_state_journal(
            self.store.as_ref(),
            &mut actor_state,
            &growth_config,
            last_state_journal_id,
        )
        .await?;

        let shared = Arc::new(SharedState {
            actor: RwLock::new(actor_state),
            config: RwLock::new(growth_config),
        });

        let (event_tx, event_rx) = self
            .event_channel
            .unwrap_or_else(|| mpsc::channel(self.event_buffer));
        let (state_tx, state_rx) = mpsc::channel(64);

        let state_handle = StateHandle::new(shared.clone(), state_tx);

        let scheduler_store = self.store.clone();
        let state_task = handle::StateTask::new(
            shared.clone(),
            self.store.clone(),
            state_rx,
            last_state_journal_id,
        );

        let mind = Mind::new(
            event_rx,
            event_tx.clone(),
            state_handle.clone(),
            self.store,
            self.media_store,
            self.router,
            self.gateway,
            self.max_concurrency,
            self.max_turns,
            self.max_action_attempts,
            self.escalate_after,
            self.metrics.clone(),
        );

        let state_join = tokio::spawn(async move {
            state_task.run().await;
        });

        let mind_join = tokio::spawn(async move {
            mind.run().await;
        });
        let scheduler_join = spawn_scheduler(event_tx.clone(), scheduler_store);

        info!("actor started");

        Ok(Actor {
            event_tx,
            mind_handle: Some(mind_join),
            state_handle: Some(state_join),
            scheduler_handle: Some(scheduler_join),
            state: state_handle,
            metrics: self.metrics,
        })
    }
}

async fn replay_state_journal(
    store: &dyn Store,
    state: &mut ActorState,
    config: &GrowthConfig,
    after_id: Option<i64>,
) -> anyhow::Result<Option<i64>> {
    let mut last_id = after_id;
    loop {
        let records = store.state_journal_after(last_id, 128).await?;
        if records.is_empty() {
            break;
        }
        for record in records {
            match record.kind.as_str() {
                "delta" => {
                    let delta: Delta = serde_json::from_value(record.payload)?;
                    state.apply_delta(&delta, config);
                }
                "idle_tick" => {
                    let elapsed_secs = record
                        .payload
                        .get("elapsed_secs")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    state.tick_idle(elapsed_secs);
                }
                "relationship_config" => {
                    let Some(person_id) = record
                        .payload
                        .get("person_id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                    else {
                        warn!(
                            journal_id = record.id,
                            "skipping malformed relationship_config journal record"
                        );
                        last_id = Some(record.id);
                        continue;
                    };
                    let authority = record
                        .payload
                        .get("authority")
                        .and_then(serde_json::Value::as_str)
                        .and_then(Authority::parse);
                    state.set_relationship_config(&PersonId(person_id.to_string()), authority);
                }
                "person_context_merge" => {
                    let Some(from) = record
                        .payload
                        .get("from_person_id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                    else {
                        warn!(
                            journal_id = record.id,
                            "skipping malformed person_context_merge journal record"
                        );
                        last_id = Some(record.id);
                        continue;
                    };
                    let Some(into) = record
                        .payload
                        .get("into_person_id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                    else {
                        warn!(
                            journal_id = record.id,
                            "skipping malformed person_context_merge journal record"
                        );
                        last_id = Some(record.id);
                        continue;
                    };
                    state.merge_person_context(
                        &PersonId(from.to_string()),
                        &PersonId(into.to_string()),
                    );
                }
                kind => {
                    warn!(
                        journal_id = record.id,
                        kind, "skipping unknown state journal record"
                    );
                }
            }
            last_id = Some(record.id);
        }
    }
    Ok(last_id)
}

impl Actor {
    pub fn builder(store: Arc<dyn Store>, router: Arc<InferenceRouter>) -> ActorBuilder {
        ActorBuilder::new(store, router)
    }

    pub async fn send_event(&self, event: WakeEvent) -> anyhow::Result<()> {
        self.event_tx.send(event).await.map_err(|_| {
            self.metrics.record_event_dropped();
            anyhow::anyhow!("actor event channel closed")
        })?;
        self.metrics.set_event_queue_depth(
            self.event_tx
                .max_capacity()
                .saturating_sub(self.event_tx.capacity()) as u64,
        );
        Ok(())
    }

    pub fn event_sender(&self) -> mpsc::Sender<WakeEvent> {
        self.event_tx.clone()
    }

    pub fn state(&self) -> &StateHandle {
        &self.state
    }

    pub fn metrics_snapshot(&self) -> ActorMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        info!("actor shutdown requested");

        if let Err(e) = self.event_tx.send(WakeEvent::Shutdown).await {
            error!(%e, "failed to send shutdown event");
        }

        if let Some(handle) = self.mind_handle.take() {
            handle.await.ok();
        }

        if let Some(handle) = self.scheduler_handle.take() {
            handle.abort();
        }

        drop(self.state);

        if let Some(handle) = self.state_handle.take() {
            handle.await.ok();
        }

        info!("actor shut down");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
