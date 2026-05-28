use crate::identity::{RelationSource, RelationStatus, SocialRelation};
use crate::state::{ActorState, Authority, Delta, GrowthConfig};
use crate::store::{ActorSnapshot, Store};
use protocol::PersonId;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

const SOCIAL_TRUST_GRAPH_MAX_NODES: usize = 128;

pub struct SharedState {
    pub actor: RwLock<ActorState>,
    pub config: RwLock<GrowthConfig>,
}

#[derive(Clone)]
pub struct StateHandle {
    pub shared: Arc<SharedState>,
    state_tx: mpsc::Sender<StateCommand>,
}

impl StateHandle {
    pub fn new(shared: Arc<SharedState>, state_tx: mpsc::Sender<StateCommand>) -> Self {
        Self { shared, state_tx }
    }

    pub fn read_state(&self) -> std::sync::RwLockReadGuard<'_, ActorState> {
        self.shared.actor.read().unwrap()
    }

    pub fn read_config(&self) -> std::sync::RwLockReadGuard<'_, GrowthConfig> {
        self.shared.config.read().unwrap()
    }

    pub async fn send_delta(&self, delta: Delta) {
        self.state_tx.send(StateCommand::Delta(delta)).await.ok();
    }

    pub async fn tick_idle(&self, elapsed_secs: f64) {
        self.state_tx
            .send(StateCommand::IdleTick { elapsed_secs })
            .await
            .ok();
    }

    pub async fn set_relationship_config(&self, person: &PersonId, authority: Option<Authority>) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = StateCommand::SetRelationshipConfig {
            person: person.clone(),
            authority,
            ack: Some(ack_tx),
        };
        if self.state_tx.send(command).await.is_ok() {
            ack_rx.await.ok();
        }
    }

    pub async fn merge_person_context(&self, from: &PersonId, into: &PersonId) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = StateCommand::MergePersonContext {
            from: from.clone(),
            into: into.clone(),
            ack: Some(ack_tx),
        };
        if self.state_tx.send(command).await.is_ok() {
            ack_rx.await.ok();
        }
    }
}

pub enum StateCommand {
    Delta(Delta),
    IdleTick {
        elapsed_secs: f64,
    },
    SetRelationshipConfig {
        person: PersonId,
        authority: Option<Authority>,
        ack: Option<oneshot::Sender<()>>,
    },
    MergePersonContext {
        from: PersonId,
        into: PersonId,
        ack: Option<oneshot::Sender<()>>,
    },
}

impl StateCommand {
    fn acknowledge(&mut self) {
        match self {
            StateCommand::SetRelationshipConfig { ack, .. }
            | StateCommand::MergePersonContext { ack, .. } => {
                if let Some(ack) = ack.take() {
                    ack.send(()).ok();
                }
            }
            _ => {}
        }
    }
}

pub struct StateTask {
    shared: Arc<SharedState>,
    store: Arc<dyn Store>,
    state_rx: mpsc::Receiver<StateCommand>,
    dirty: bool,
    last_journal_id: Option<i64>,
}

impl StateTask {
    pub fn new(
        shared: Arc<SharedState>,
        store: Arc<dyn Store>,
        state_rx: mpsc::Receiver<StateCommand>,
        last_journal_id: Option<i64>,
    ) -> Self {
        Self {
            shared,
            store,
            state_rx,
            dirty: false,
            last_journal_id,
        }
    }

    pub async fn run(mut self) {
        let save_interval = tokio::time::Duration::from_secs(300);
        loop {
            tokio::select! {
                maybe_command = self.state_rx.recv() => {
                    match maybe_command {
                        Some(command) => {
                            let mut batch = vec![command];
                            while let Ok(command) = self.state_rx.try_recv() {
                                batch.push(command);
                            }
                            self.apply_batch(batch).await;
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

    async fn apply_batch(&mut self, batch: Vec<StateCommand>) {
        let mut applied = 0usize;
        for command in batch {
            let mut command = self.prepare_command(command).await;
            let journal_id = match self.persist_command(&command).await {
                Ok(journal_id) => journal_id,
                Err(e) => {
                    warn!(%e, "failed to persist state journal entry; skipping state command");
                    command.acknowledge();
                    continue;
                }
            };

            let config = self.shared.config.read().unwrap().clone();
            {
                let mut state = self.shared.actor.write().unwrap();
                match &command {
                    StateCommand::Delta(delta) => state.apply_delta(delta, &config),
                    StateCommand::IdleTick { elapsed_secs } => state.tick_idle(*elapsed_secs),
                    StateCommand::SetRelationshipConfig {
                        person, authority, ..
                    } => {
                        state.set_relationship_config(person, authority.clone());
                    }
                    StateCommand::MergePersonContext { from, into, .. } => {
                        state.merge_person_context(from, into);
                    }
                }
            }
            command.acknowledge();
            if let Some(journal_id) = journal_id {
                self.last_journal_id = Some(journal_id);
            }
            applied += 1;
        }
        if applied > 0 {
            self.dirty = true;
            info!(count = applied, "applied state commands");
        }
    }

    async fn prepare_command(&self, command: StateCommand) -> StateCommand {
        match command {
            StateCommand::Delta(mut delta) => {
                for change in &mut delta.relationship_changes {
                    if change.trust_delta > 0.0 && change.trust_ceiling.is_none() {
                        change.trust_ceiling =
                            Some(self.relationship_trust_ceiling(&change.person).await);
                    }
                }
                StateCommand::Delta(delta)
            }
            other => other,
        }
    }

    async fn relationship_trust_ceiling(&self, person: &PersonId) -> f32 {
        let (authority, current_trust, chosen_person_ids) = {
            let actor = self.shared.actor.read().unwrap();
            let relationship = actor.bonds.get(person);
            let chosen_person_ids = actor
                .bonds
                .iter()
                .filter(|(_, rel)| rel.authority == Authority::ChosenPerson)
                .map(|(person, _)| person.clone())
                .collect::<Vec<_>>();
            (
                relationship
                    .map(|rel| rel.authority.clone())
                    .unwrap_or(Authority::Default),
                relationship
                    .map(|rel| rel.trust)
                    .unwrap_or_else(|| crate::state::Relationship::default().trust),
                chosen_person_ids,
            )
        };

        match authority {
            Authority::ChosenPerson
            | Authority::Trusted
            | Authority::Restricted
            | Authority::Blocked => authority.trust_ceiling(),
            Authority::Default => {
                if chosen_person_ids
                    .iter()
                    .any(|chosen_person| chosen_person == person)
                    || self
                        .social_graph_connects_to_chosen_person(person, &chosen_person_ids)
                        .await
                {
                    Authority::Default.trust_ceiling()
                } else {
                    current_trust.clamp(0.0, Authority::Default.trust_ceiling())
                }
            }
        }
    }

    async fn social_graph_connects_to_chosen_person(
        &self,
        person: &PersonId,
        chosen_person_ids: &[PersonId],
    ) -> bool {
        if chosen_person_ids.is_empty() {
            return false;
        }
        if chosen_person_ids
            .iter()
            .any(|chosen_person| chosen_person == person)
        {
            return true;
        }

        let chosen_person_ids = chosen_person_ids.iter().cloned().collect::<HashSet<_>>();
        let mut visited = HashSet::from([person.clone()]);
        let mut queue = VecDeque::from([person.clone()]);

        while let Some(current) = queue.pop_front() {
            let relations = match self.store.get_relations(&current).await {
                Ok(relations) => relations,
                Err(e) => {
                    warn!(
                        %e,
                        person = %current.0,
                        "failed to read social graph while computing trust ceiling"
                    );
                    return false;
                }
            };

            for relation in relations.into_iter().filter(relation_counts_for_trust_path) {
                let Some(next) = other_social_relation_person(&relation, &current) else {
                    continue;
                };
                if !visited.insert(next.clone()) {
                    continue;
                }
                if chosen_person_ids.contains(&next) {
                    return true;
                }
                if visited.len() >= SOCIAL_TRUST_GRAPH_MAX_NODES {
                    warn!(
                        start = %person.0,
                        limit = SOCIAL_TRUST_GRAPH_MAX_NODES,
                        "stopped social graph trust traversal at node limit"
                    );
                    return false;
                }
                queue.push_back(next);
            }
        }

        false
    }

    async fn persist_command(&self, command: &StateCommand) -> anyhow::Result<Option<i64>> {
        let (kind, payload) = match command {
            StateCommand::Delta(delta) => ("delta", serde_json::to_value(delta)?),
            StateCommand::IdleTick { elapsed_secs } => (
                "idle_tick",
                serde_json::json!({ "elapsed_secs": elapsed_secs }),
            ),
            StateCommand::SetRelationshipConfig {
                person, authority, ..
            } => (
                "relationship_config",
                serde_json::json!({
                    "person_id": person.0.as_str(),
                    "authority": authority.as_ref().map(Authority::as_str),
                }),
            ),
            StateCommand::MergePersonContext { from, into, .. } => (
                "person_context_merge",
                serde_json::json!({
                    "from_person_id": from.0.as_str(),
                    "into_person_id": into.0.as_str(),
                }),
            ),
        };
        self.store
            .append_state_journal(kind, &payload, now())
            .await
            .map(Some)
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
                last_state_journal_id: self.last_journal_id,
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

fn relation_counts_for_trust_path(relation: &SocialRelation) -> bool {
    matches!(
        relation.status,
        RelationStatus::Stated | RelationStatus::Confirmed
    ) && !matches!(relation.source_kind, RelationSource::Inferred)
        && relation.confidence >= 0.5
}

fn other_social_relation_person(relation: &SocialRelation, person: &PersonId) -> Option<PersonId> {
    if relation.person_a == *person {
        Some(relation.person_b.clone())
    } else if relation.person_b == *person {
        Some(relation.person_a.clone())
    } else {
        None
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Relation, RelationSource, RelationStatus};
    use crate::state::{CoreTraits, Interest};
    use crate::store::{SqliteStore, Store};

    #[tokio::test]
    async fn relationship_config_updates_shared_state_and_journal() {
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(CoreTraits::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (tx, rx) = mpsc::channel(1);
        let state_task = StateTask::new(shared.clone(), store.clone(), rx, None);
        let state_join = tokio::spawn(async move {
            state_task.run().await;
        });
        let handle = StateHandle::new(shared.clone(), tx);
        let person = PersonId("person-chosen_person".into());

        handle
            .set_relationship_config(&person, Some(Authority::ChosenPerson))
            .await;

        assert_eq!(
            shared.actor.read().unwrap().bonds[&person].authority,
            Authority::ChosenPerson
        );
        let records = store.state_journal_after(None, 10).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, "relationship_config");
        assert_eq!(records[0].payload["person_id"], "person-chosen_person");
        assert_eq!(records[0].payload["authority"], "chosen_person");

        drop(handle);
        state_join.await.unwrap();
    }

    #[tokio::test]
    async fn person_context_merge_updates_shared_state_and_journal() {
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(CoreTraits::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (tx, rx) = mpsc::channel(1);
        let state_task = StateTask::new(shared.clone(), store.clone(), rx, None);
        let state_join = tokio::spawn(async move {
            state_task.run().await;
        });
        let handle = StateHandle::new(shared.clone(), tx);
        let from = PersonId("person-claimant".into());
        let into = PersonId("person-verified".into());
        {
            let mut actor = shared.actor.write().unwrap();
            actor.set_relationship_config(&from, Some(Authority::Trusted));
        }

        handle.merge_person_context(&from, &into).await;

        let actor = shared.actor.read().unwrap();
        assert!(!actor.bonds.contains_key(&from));
        assert!(actor.bonds.contains_key(&into));
        drop(actor);
        let records = store.state_journal_after(None, 10).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, "person_context_merge");
        assert_eq!(records[0].payload["from_person_id"], "person-claimant");
        assert_eq!(records[0].payload["into_person_id"], "person-verified");

        drop(handle);
        state_join.await.unwrap();
    }

    #[tokio::test]
    async fn idle_tick_decays_state_and_journals() {
        let mut actor = ActorState::new(CoreTraits::default());
        actor.affect.valence = 0.9;
        actor.affect.arousal = 0.8;
        actor.affect.dominance = 0.9;
        actor.interests.push(Interest {
            topic: "deployment systems".into(),
            intensity: 0.8,
            origin: "test".into(),
            origin_person: None,
            last_engaged: 1000,
        });
        let shared = Arc::new(SharedState {
            actor: RwLock::new(actor),
            config: RwLock::new(GrowthConfig::default()),
        });
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (tx, rx) = mpsc::channel(1);
        let state_task = StateTask::new(shared.clone(), store.clone(), rx, None);
        let state_join = tokio::spawn(async move {
            state_task.run().await;
        });
        let handle = StateHandle::new(shared.clone(), tx);

        handle.tick_idle(600.0).await;
        drop(handle);
        state_join.await.unwrap();

        let actor = shared.actor.read().unwrap();
        assert!(actor.affect.valence < 0.9);
        assert!(actor.affect.arousal < 0.8);
        assert!(actor.affect.dominance < 0.9);
        assert_eq!(actor.interests.len(), 1);
        assert!(actor.interests[0].intensity < 0.8);
        drop(actor);

        let records = store.state_journal_after(None, 10).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, "idle_tick");
        assert_eq!(records[0].payload["elapsed_secs"], 600.0);
    }

    #[tokio::test]
    async fn state_task_blocks_positive_trust_for_unconnected_default_person() {
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(CoreTraits::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let (_tx, rx) = mpsc::channel(1);
        let mut task = StateTask::new(shared.clone(), store.clone(), rx, None);
        let person = PersonId("person-stranger".into());

        task.apply_batch(vec![StateCommand::Delta(Delta {
            relationship_changes: vec![crate::state::RelationshipChange {
                person: person.clone(),
                trust_delta: 0.05,
                trust_ceiling: None,
                familiarity_delta: 0.0,
                valence_delta: 0.0,
                proactive_consent: None,
                response_cadence: None,
                channel_preference: None,
                interaction: None,
            }],
            ..Delta::default()
        })])
        .await;

        let actor = shared.actor.read().unwrap();
        assert_eq!(actor.bonds[&person].trust, 0.3);
    }

    #[tokio::test]
    async fn state_task_allows_trust_for_chosen_person_connected_social_path() {
        let chosen_person = PersonId("person-chosen_person".into());
        let middle = PersonId("person-middle".into());
        let person = PersonId("person-connected".into());
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(CoreTraits::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        {
            let mut actor = shared.actor.write().unwrap();
            actor.set_relationship_config(&chosen_person, Some(Authority::ChosenPerson));
        }

        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        store
            .upsert_relation(&SocialRelation {
                person_a: chosen_person.clone(),
                person_b: middle.clone(),
                relation: Relation::Friend,
                direction: Relation::Friend.default_direction(),
                confidence: 0.9,
                status: RelationStatus::Confirmed,
                evidence: Some(serde_json::json!({"source": "test"})),
                source_kind: RelationSource::ChosenPersonConfirmed,
                asserted_by: Some(chosen_person.clone()),
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        store
            .upsert_relation(&SocialRelation {
                person_a: middle.clone(),
                person_b: person.clone(),
                relation: Relation::Coworker,
                direction: Relation::Coworker.default_direction(),
                confidence: 0.8,
                status: RelationStatus::Stated,
                evidence: Some(serde_json::json!({"source": "test"})),
                source_kind: RelationSource::Stated,
                asserted_by: Some(middle.clone()),
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();

        let (_tx, rx) = mpsc::channel(1);
        let mut task = StateTask::new(shared.clone(), store, rx, None);
        task.apply_batch(vec![StateCommand::Delta(Delta {
            relationship_changes: vec![crate::state::RelationshipChange {
                person: person.clone(),
                trust_delta: 0.05,
                trust_ceiling: None,
                familiarity_delta: 0.0,
                valence_delta: 0.0,
                proactive_consent: None,
                response_cadence: None,
                channel_preference: None,
                interaction: None,
            }],
            ..Delta::default()
        })])
        .await;

        let actor = shared.actor.read().unwrap();
        assert!(actor.bonds[&person].trust > 0.3);
    }
}
