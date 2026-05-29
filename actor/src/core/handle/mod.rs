use crate::identity::{RelationSource, RelationStatus, SocialRelation};
use crate::state::{ActorState, AdoptionRitualState, Delta, GrowthConfig, RelationshipStanding};
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

    pub async fn set_relationship_config(
        &self,
        person: &PersonId,
        relationship_standing: Option<RelationshipStanding>,
    ) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = StateCommand::SetRelationshipConfig {
            person: person.clone(),
            relationship_standing,
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

    pub async fn set_adoption_state(&self, person: &PersonId, state: AdoptionRitualState) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = StateCommand::SetAdoptionState {
            person: person.clone(),
            state,
            updated_at: now(),
            ack: Some(ack_tx),
        };
        if self.state_tx.send(command).await.is_ok() {
            ack_rx.await.ok();
        }
    }

    pub async fn complete_adoption(&self, person: &PersonId) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = StateCommand::CompleteAdoption {
            person: person.clone(),
            updated_at: now(),
            ack: Some(ack_tx),
        };
        if self.state_tx.send(command).await.is_ok() {
            ack_rx.await.ok();
        }
    }

    pub async fn settle_completed_adoption_marker(&self, person: &PersonId) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let command = StateCommand::SettleCompletedAdoptionMarker {
            person: person.clone(),
            updated_at: now(),
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
        relationship_standing: Option<RelationshipStanding>,
        ack: Option<oneshot::Sender<()>>,
    },
    MergePersonContext {
        from: PersonId,
        into: PersonId,
        ack: Option<oneshot::Sender<()>>,
    },
    SetAdoptionState {
        person: PersonId,
        state: AdoptionRitualState,
        updated_at: i64,
        ack: Option<oneshot::Sender<()>>,
    },
    CompleteAdoption {
        person: PersonId,
        updated_at: i64,
        ack: Option<oneshot::Sender<()>>,
    },
    SettleCompletedAdoptionMarker {
        person: PersonId,
        updated_at: i64,
        ack: Option<oneshot::Sender<()>>,
    },
}

impl StateCommand {
    fn acknowledge(&mut self) {
        match self {
            StateCommand::SetRelationshipConfig { ack, .. }
            | StateCommand::MergePersonContext { ack, .. }
            | StateCommand::SetAdoptionState { ack, .. }
            | StateCommand::CompleteAdoption { ack, .. }
            | StateCommand::SettleCompletedAdoptionMarker { ack, .. } => {
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
                        person,
                        relationship_standing,
                        ..
                    } => {
                        state.set_relationship_config(person, relationship_standing.clone());
                    }
                    StateCommand::MergePersonContext { from, into, .. } => {
                        state.merge_person_context(from, into);
                    }
                    StateCommand::SetAdoptionState {
                        person,
                        state: ritual_state,
                        updated_at,
                        ..
                    } => {
                        state.set_adoption_state(person, ritual_state.clone(), *updated_at);
                    }
                    StateCommand::CompleteAdoption {
                        person, updated_at, ..
                    } => {
                        state.complete_adoption(person, *updated_at);
                    }
                    StateCommand::SettleCompletedAdoptionMarker {
                        person, updated_at, ..
                    } => {
                        state.settle_completed_adoption_marker(person, *updated_at);
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
        let (relationship_standing, current_trust, chosen_human_ids) = {
            let actor = self.shared.actor.read().unwrap();
            let relationship = actor.bonds.get(person);
            let chosen_human_ids = actor
                .bonds
                .iter()
                .filter(|(_, rel)| rel.relationship_standing == RelationshipStanding::ChosenHuman)
                .map(|(person, _)| person.clone())
                .collect::<Vec<_>>();
            (
                relationship
                    .map(|rel| rel.relationship_standing.clone())
                    .unwrap_or(RelationshipStanding::Default),
                relationship
                    .map(|rel| rel.trust)
                    .unwrap_or_else(|| crate::state::Relationship::default().trust),
                chosen_human_ids,
            )
        };

        match relationship_standing {
            RelationshipStanding::ChosenHuman
            | RelationshipStanding::Trusted
            | RelationshipStanding::Restricted
            | RelationshipStanding::Blocked => relationship_standing.trust_ceiling(),
            RelationshipStanding::Default => {
                if chosen_human_ids
                    .iter()
                    .any(|chosen_human| chosen_human == person)
                    || self
                        .social_graph_connects_to_chosen_human(person, &chosen_human_ids)
                        .await
                {
                    RelationshipStanding::Default.trust_ceiling()
                } else {
                    current_trust.clamp(0.0, RelationshipStanding::Default.trust_ceiling())
                }
            }
        }
    }

    async fn social_graph_connects_to_chosen_human(
        &self,
        person: &PersonId,
        chosen_human_ids: &[PersonId],
    ) -> bool {
        if chosen_human_ids.is_empty() {
            return false;
        }
        if chosen_human_ids
            .iter()
            .any(|chosen_human| chosen_human == person)
        {
            return true;
        }

        let chosen_human_ids = chosen_human_ids.iter().cloned().collect::<HashSet<_>>();
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
                if chosen_human_ids.contains(&next) {
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
                person,
                relationship_standing,
                ..
            } => (
                "relationship_config",
                serde_json::json!({
                    "person_id": person.0.as_str(),
                    "relationship_standing": relationship_standing.as_ref().map(RelationshipStanding::as_str),
                }),
            ),
            StateCommand::MergePersonContext { from, into, .. } => (
                "person_context_merge",
                serde_json::json!({
                    "from_person_id": from.0.as_str(),
                    "into_person_id": into.0.as_str(),
                }),
            ),
            StateCommand::SetAdoptionState {
                person,
                state,
                updated_at,
                ..
            } => (
                "adoption_state",
                serde_json::json!({
                    "person_id": person.0.as_str(),
                    "state": state.as_str(),
                    "updated_at": updated_at,
                }),
            ),
            StateCommand::CompleteAdoption {
                person, updated_at, ..
            } => (
                "adoption_complete",
                serde_json::json!({
                    "person_id": person.0.as_str(),
                    "updated_at": updated_at,
                }),
            ),
            StateCommand::SettleCompletedAdoptionMarker {
                person, updated_at, ..
            } => (
                "adoption_marker_settled",
                serde_json::json!({
                    "person_id": person.0.as_str(),
                    "updated_at": updated_at,
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
mod tests;
