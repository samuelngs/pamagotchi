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
mod tests {
    use super::super::event::claim_and_send_persisted_event;
    use super::super::scheduler::{
        claim_and_send_due_intent, drain_due_events, drain_due_intents, emit_due_consolidation,
        take_due_scheduler_elapsed,
    };
    use super::*;
    use crate::core::FiredIntent;
    use crate::state::{CoreTraits, RelationshipChange};
    use crate::store::{ActorSnapshot, EventInboxRecord, IntentRecord, SqliteStore};
    use async_trait::async_trait;
    use inference::{
        AssistantMessage, Capability, ChatRequest, ChatResponse, ChatStream, FinishReason,
        InferenceEndpoint, InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge,
        Reasoning, SamplingConfig, Usage,
    };
    use protocol::{ConversationId, InboundMessage, PersonId};

    fn inbound() -> InboundMessage {
        InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: Some("Sam".into()),
            reply_external_id: "local".into(),
            conversation: ConversationId("relay:local".into()),
            group: None,
            identity: None,
            profile: None,
            person: None,
            content: "hello".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        }
    }

    struct NoopBridge;

    #[async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: AssistantMessage {
                    text: Some(String::new()),
                    reasoning_content: None,
                    tool_calls: vec![],
                },
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("noop bridge is not used by actor replay tests")
        }
    }

    fn test_router() -> InferenceRouter {
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn scheduler_drains_due_message_events_once() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let message = inbound();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-message".into(),
                kind: "message".into(),
                payload: serde_json::to_value(&message).unwrap(),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 800,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

        match rx.recv().await.unwrap() {
            WakeEvent::Message(msg) => assert_eq!(msg.message_id, "msg-1"),
            _ => panic!("expected deferred message event"),
        }
        assert!(store.due_events(1001, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn claimed_persisted_event_is_not_emitted_twice() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let message = inbound();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-message".into(),
                kind: "message".into(),
                payload: serde_json::to_value(&message).unwrap(),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();
        assert!(store.mark_event_fired("event-message", 1000).await.unwrap());

        let (tx, mut rx) = mpsc::channel(1);
        assert!(
            claim_and_send_persisted_event(
                &tx,
                store_dyn.as_ref(),
                "event-message",
                1001,
                WakeEvent::Message(message),
                "test duplicate"
            )
            .await
        );

        assert!(rx.try_recv().is_err());
        assert!(store.due_events(1001, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn persisted_event_stays_pending_when_handoff_channel_is_closed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let message = inbound();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-message".into(),
                kind: "message".into(),
                payload: serde_json::to_value(&message).unwrap(),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        assert!(
            !claim_and_send_persisted_event(
                &tx,
                store_dyn.as_ref(),
                "event-message",
                1000,
                WakeEvent::Message(message),
                "test closed channel"
            )
            .await
        );

        let due = store.due_events(1000, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "event-message");
    }

    #[tokio::test]
    async fn scheduler_leaves_due_event_pending_when_actor_channel_is_closed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let message = inbound();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-message".into(),
                kind: "message".into(),
                payload: serde_json::to_value(&message).unwrap(),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        assert!(!drain_due_events(&tx, store_dyn, 1000, 10).await);

        let due = store.due_events(1001, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "event-message");
    }

    #[tokio::test]
    async fn scheduler_replays_message_edit_and_delete_events() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let edited = crate::core::MessageEditedEvent {
            conversation: conversation.clone(),
            gateway_id: "relay".into(),
            message_id: "msg-1".into(),
            content: "edited text".into(),
            edited_at: 1100,
        };
        let deleted = crate::core::MessageDeletedEvent {
            conversation: conversation.clone(),
            gateway_id: "relay".into(),
            message_id: "msg-2".into(),
            deleted_at: 1200,
        };
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-edit".into(),
                kind: "message_edited".into(),
                payload: serde_json::to_value(&edited).unwrap(),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-delete".into(),
                kind: "message_deleted".into(),
                payload: serde_json::to_value(&deleted).unwrap(),
                status: "pending".into(),
                due_at: 1001,
                attempts: 0,
                dedupe_key: None,
                created_at: 901,
                updated_at: 901,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(2);
        assert!(drain_due_events(&tx, store_dyn, 1001, 10).await);

        match rx.recv().await.unwrap() {
            WakeEvent::MessageEdited {
                conversation,
                gateway_id,
                message_id,
                content,
                edited_at,
            } => {
                assert_eq!(conversation.0, "relay:local");
                assert_eq!(gateway_id, "relay");
                assert_eq!(message_id, "msg-1");
                assert_eq!(content, "edited text");
                assert_eq!(edited_at, 1100);
            }
            _ => panic!("expected message edit event"),
        }
        match rx.recv().await.unwrap() {
            WakeEvent::MessageDeleted {
                conversation,
                gateway_id,
                message_id,
                deleted_at,
            } => {
                assert_eq!(conversation.0, "relay:local");
                assert_eq!(gateway_id, "relay");
                assert_eq!(message_id, "msg-2");
                assert_eq!(deleted_at, 1200);
            }
            _ => panic!("expected message delete event"),
        }
        assert!(store.due_events(1002, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn scheduler_leaves_due_intent_active_when_actor_channel_is_closed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        store
            .create_intent(&IntentRecord {
                id: "intent-message".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Check in".into(),
                person: Some(PersonId("person-sam".into())),
                profile: None,
                conversation: Some(ConversationId("relay:local".into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                owner_approved: false,
            })
            .await
            .unwrap();

        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        assert!(!drain_due_intents(&tx, store_dyn, 1000, 10).await);

        let due = store.due_intents(1001, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "intent-message");
        assert_eq!(due[0].status, "active");
    }

    #[tokio::test]
    async fn claimed_due_intent_is_not_emitted_twice() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        store
            .create_intent(&IntentRecord {
                id: "intent-message".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Check in".into(),
                person: Some(PersonId("person-sam".into())),
                profile: None,
                conversation: Some(ConversationId("relay:local".into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                owner_approved: false,
            })
            .await
            .unwrap();
        assert!(
            store
                .mark_intent_fired("intent-message", 1000)
                .await
                .unwrap()
        );

        let intent = store.get_intent("intent-message").await.unwrap().unwrap();
        let (tx, mut rx) = mpsc::channel(1);
        assert!(claim_and_send_due_intent(&tx, store_dyn.as_ref(), intent, 1001).await);

        assert!(rx.try_recv().is_err());
        assert!(store.due_intents(1001, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn scheduler_drains_due_intent_after_actor_handoff() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        store
            .create_intent(&IntentRecord {
                id: "intent-message".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Check in".into(),
                person: Some(PersonId("person-sam".into())),
                profile: None,
                conversation: Some(ConversationId("relay:local".into())),
                fire_at: Some(1000),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 900,
                updated_at: 900,
                last_fired_at: None,
                owner_approved: true,
            })
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        assert!(drain_due_intents(&tx, store_dyn, 1000, 10).await);

        match rx.recv().await.unwrap() {
            WakeEvent::IntentFired(intent) => {
                assert_eq!(intent.id, "intent-message");
                assert_eq!(intent.task, "Check in");
                assert_eq!(
                    intent.conversation,
                    Some(ConversationId("relay:local".into()))
                );
                assert_eq!(intent.person, Some(PersonId("person-sam".into())));
                assert_eq!(intent.scheduled_at, Some(900));
                assert!(intent.owner_approved);
            }
            _ => panic!("expected fired intent"),
        }
        assert!(store.due_intents(1001, 10).await.unwrap().is_empty());
        assert_eq!(
            store
                .get_intent("intent-message")
                .await
                .unwrap()
                .unwrap()
                .status,
            "fired"
        );
    }

    #[tokio::test]
    async fn scheduler_preserves_owner_approval_on_deferred_intent_events() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let intent = FiredIntent {
            id: "intent-owner-approved".into(),
            task: "Check in".into(),
            conversation: Some(ConversationId("relay:local".into())),
            person: Some(PersonId("person-sam".into())),
            scheduled_at: Some(900),
            owner_approved: true,
            defer_count: 1,
        };
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-intent".into(),
                kind: "intent_fired".into(),
                payload: serde_json::to_value(&intent).unwrap(),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

        match rx.recv().await.unwrap() {
            WakeEvent::IntentFired(intent) => {
                assert_eq!(intent.id, "intent-owner-approved");
                assert_eq!(intent.scheduled_at, Some(900));
                assert!(intent.owner_approved);
                assert_eq!(intent.defer_count, 1);
            }
            _ => panic!("expected deferred intent event"),
        }
    }

    #[tokio::test]
    async fn scheduler_drains_due_consolidation_events_once() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-consolidation".into(),
                kind: "consolidation_due".into(),
                payload: serde_json::json!({}),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: Some("consolidation-due".into()),
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

        assert!(matches!(rx.recv().await, Some(WakeEvent::ConsolidationDue)));
        assert!(store.due_events(1001, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn scheduler_marks_malformed_due_events_failed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-malformed-message".into(),
                kind: "message".into(),
                payload: serde_json::json!({"malformed": true}),
                status: "pending".into(),
                due_at: 1000,
                attempts: 0,
                dedupe_key: None,
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        assert!(drain_due_events(&tx, store_dyn, 1000, 10).await);

        assert!(rx.try_recv().is_err());
        assert!(store.due_events(1001, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn scheduler_emits_due_consolidation_event() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (tx, mut rx) = mpsc::channel(1);

        assert!(emit_due_consolidation(&tx, store_dyn, 1000).await);
        assert!(matches!(rx.recv().await, Some(WakeEvent::ConsolidationDue)));
        assert!(
            store
                .pending_events_by_kind("consolidation_due", 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn scheduler_leaves_periodic_consolidation_pending_when_actor_channel_is_closed() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        assert!(!emit_due_consolidation(&tx, store_dyn, 1000).await);
        let pending = store
            .pending_events_by_kind("consolidation_due", 10)
            .await
            .unwrap();

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "consolidation_due");
        assert_eq!(pending[0].dedupe_key.as_deref(), Some("consolidation-due"));
    }

    #[tokio::test]
    async fn scheduler_does_not_duplicate_pending_periodic_consolidation() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        store
            .enqueue_event(&EventInboxRecord {
                id: "event-consolidation-existing".into(),
                kind: "consolidation_due".into(),
                payload: serde_json::json!({}),
                status: "pending".into(),
                due_at: 2000,
                attempts: 0,
                dedupe_key: Some("consolidation-due".into()),
                created_at: 900,
                updated_at: 900,
                fired_at: None,
                last_error: None,
            })
            .await
            .unwrap();
        let (tx, mut rx) = mpsc::channel(1);

        assert!(emit_due_consolidation(&tx, store_dyn, 1000).await);

        assert!(rx.try_recv().is_err());
        let pending = store
            .pending_events_by_kind("consolidation_due", 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "event-consolidation-existing");
    }

    #[test]
    fn scheduler_elapsed_uses_actual_monotonic_gap() {
        let mut elapsed = 0.0;

        assert_eq!(take_due_scheduler_elapsed(&mut elapsed, 30.0, 300.0), None);
        assert_eq!(elapsed, 30.0);
        assert_eq!(
            take_due_scheduler_elapsed(&mut elapsed, 420.0, 300.0),
            Some(450.0)
        );
        assert_eq!(elapsed, 0.0);
    }

    #[tokio::test]
    async fn actor_replays_state_journal_after_snapshot() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        store
            .save_snapshot(&ActorSnapshot {
                state: ActorState::new(CoreTraits::default()),
                config: GrowthConfig::default(),
                saved_at: 1000,
                last_state_journal_id: Some(0),
            })
            .await
            .unwrap();

        let person = PersonId("person-journal".into());
        let delta = Delta {
            relationship_changes: vec![RelationshipChange {
                person: person.clone(),
                trust_delta: 0.0,
                trust_ceiling: None,
                familiarity_delta: 1.0,
                valence_delta: 0.0,
                proactive_consent: None,
                response_cadence: None,
                channel_preference: None,
                interaction: Some(crate::state::RelationshipInteraction::Inbound),
            }],
            ..Delta::default()
        };
        store
            .append_state_journal("delta", &serde_json::to_value(delta).unwrap(), 1001)
            .await
            .unwrap();

        let store_dyn: Arc<dyn Store> = store;
        let actor = ActorBuilder::new(store_dyn, Arc::new(test_router()))
            .build()
            .await
            .unwrap();
        {
            let state = actor.state().read_state();
            let relationship = state.bonds.get(&person).expect("journal replayed bond");
            assert_eq!(relationship.interaction_count, 1);
            assert_eq!(relationship.inbound_count, 1);
            assert!(relationship.familiarity > 0.0);
        }

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn actor_replays_relationship_config_and_person_merge_journal_records() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        store
            .save_snapshot(&ActorSnapshot {
                state: ActorState::new(CoreTraits::default()),
                config: GrowthConfig::default(),
                saved_at: 1000,
                last_state_journal_id: Some(0),
            })
            .await
            .unwrap();

        store
            .append_state_journal(
                "relationship_config",
                &serde_json::json!({
                    "person_id": "person-claimant",
                    "authority": "trusted",
                }),
                1001,
            )
            .await
            .unwrap();
        store
            .append_state_journal(
                "relationship_config",
                &serde_json::json!({
                    "person_id": "person-owner",
                    "authority": "owner",
                }),
                1002,
            )
            .await
            .unwrap();
        store
            .append_state_journal(
                "person_context_merge",
                &serde_json::json!({
                    "from_person_id": "person-claimant",
                    "into_person_id": "person-owner",
                }),
                1003,
            )
            .await
            .unwrap();

        let store_dyn: Arc<dyn Store> = store;
        let actor = ActorBuilder::new(store_dyn, Arc::new(test_router()))
            .build()
            .await
            .unwrap();
        {
            let state = actor.state().read_state();
            assert!(
                !state
                    .bonds
                    .contains_key(&PersonId("person-claimant".into()))
            );
            assert_eq!(
                state.bonds[&PersonId("person-owner".into())].authority,
                crate::state::Authority::Owner
            );
        }

        actor.shutdown().await.unwrap();
    }
}
