mod evaluate;
mod respond;
mod spawn;

use super::action::{ActionId, FollowUp};
use super::event::WakeEvent;
use super::registry::ActionRegistry;
use super::handle::StateHandle;
use inference::InferenceRouter;
use gateway::GatewayRouter;
use crate::identity::{Identity, Person};
use crate::state::{Authority, Relationship};
use crate::store::Store;
use protocol::{InboundMessage, PersonId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

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
                Some(WakeEvent::ActionCompleted { action_id, outcome }) => {
                    self.handle_completed(&action_id, outcome).await;
                }
                Some(mut event) => {
                    if let WakeEvent::Message(ref mut msg) = event {
                        self.resolve_person(msg).await;
                    }
                    if let WakeEvent::Message(ref msg) = event {
                        if let Some(target) = self.registry.unreplied_in(&msg.conversation) {
                            let target_id = target.id.clone();
                            if self.registry.inject(&target_id, msg.clone()) {
                                info!(%target_id, "injected message into running action");
                                continue;
                            }
                        }
                    }
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

    async fn handle_completed(&mut self, action_id: &ActionId, outcome: super::action::Outcome) {
        self.registry.complete(action_id, outcome);

        if let Some(action) = self.registry.get(action_id) {
            if let super::action::Phase::Done { outcome } = &action.phase {
                if let Some(ref delta) = outcome.delta {
                    self.state.send_delta(delta.clone()).await;
                    info!(%action_id, "forwarded personality delta");
                }
            }
        }

        let follow_ups = self.registry.follow_ups(action_id);
        for fu in follow_ups {
            match fu {
                FollowUp::Requeue(msgs) => {
                    for msg in msgs {
                        info!(%action_id, "re-queuing source message after failed respond");
                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                }
                FollowUp::ReemitPending(msgs) => {
                    for msg in msgs {
                        info!(%action_id, "re-emitting pending message");
                        self.event_tx.send(WakeEvent::Message(msg)).await.ok();
                    }
                }
            }
        }

        let ready = self.registry.ready();
        for id in ready {
            self.launch_action(&id).await;
        }

        self.registry.gc();
    }

    async fn resolve_person(&self, msg: &mut InboundMessage) {
        if msg.person.is_some() {
            return;
        }
        if msg.gateway_id == "relay" {
            self.resolve_relay_person(msg).await;
        } else {
            self.resolve_gateway_person(msg).await;
        }
    }

    async fn resolve_relay_person(&self, msg: &mut InboundMessage) {
        if let Some(owner_id) = self.find_owner() {
            let _ = self.store.touch_person(&owner_id).await;
            self.ensure_identity_linked(&owner_id, &msg.gateway_id, &msg.external_id).await;
            msg.person = Some(owner_id);
        } else {
            let id = self.create_person_with_identity(msg, Authority::Owner).await;
            if let Some(ref id) = id {
                info!(person = %id.0, "created owner from first relay contact");
            }
            msg.person = id;
        }
    }

    async fn resolve_gateway_person(&self, msg: &mut InboundMessage) {
        match self.store.resolve_identity(&msg.gateway_id, &msg.external_id).await {
            Ok(Some(person)) => {
                let _ = self.store.touch_person(&person.id).await;
                msg.person = Some(person.id);
            }
            Ok(None) => {
                let authority = if self.find_owner().is_none() {
                    Authority::Owner
                } else {
                    Authority::Default
                };
                msg.person = self.create_person_with_identity(msg, authority).await;
            }
            Err(e) => {
                warn!("failed to resolve identity: {e}");
            }
        }
    }

    fn find_owner(&self) -> Option<PersonId> {
        let actor = self.state.read_state();
        actor.bonds
            .iter()
            .find(|(_, rel)| rel.authority == Authority::Owner)
            .map(|(id, _)| id.clone())
    }

    async fn create_person_with_identity(
        &self,
        msg: &InboundMessage,
        authority: Authority,
    ) -> Option<PersonId> {
        let id = PersonId(nanoid::nanoid!());
        let now = chrono::Utc::now().timestamp();
        let person = Person {
            id: id.clone(),
            name: None,
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
        };
        if let Err(e) = self.store.add_person(&person).await {
            warn!("failed to create person: {e}");
            return None;
        }
        let identity = Identity {
            gateway_id: msg.gateway_id.clone(),
            external_id: msg.external_id.clone(),
            display_name: None,
        };
        if let Err(e) = self.store.add_identity(&id, &identity).await {
            warn!("failed to add identity: {e}");
        }
        let mut rel = Relationship::default();
        rel.authority = authority;
        self.state.shared.actor.write().unwrap()
            .bonds.insert(id.clone(), rel);
        Some(id)
    }

    async fn ensure_identity_linked(&self, person: &PersonId, gateway_id: &str, external_id: &str) {
        if let Ok(Some(_)) = self.store.resolve_identity(gateway_id, external_id).await {
            return;
        }
        let identity = Identity {
            gateway_id: gateway_id.to_string(),
            external_id: external_id.to_string(),
            display_name: None,
        };
        let _ = self.store.add_identity(person, &identity).await;
    }

    async fn shutdown(&mut self) {
        info!("mind shutting down, cancelling all actions");
        let running: Vec<ActionId> = self
            .registry
            .running()
            .iter()
            .map(|a| a.id.clone())
            .collect();
        for id in &running {
            self.registry.cancel(id);
        }
    }
}
