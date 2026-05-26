mod evaluate;
mod respond;
mod spawn;

use super::action::{ActionId, FollowUp};
use super::event::WakeEvent;
use super::handle::StateHandle;
use super::registry::ActionRegistry;
use crate::identity::{Identity, Person, PersonProfileStatus, Profile, ResolvedActorIdentity};
use crate::state::{Authority, Relationship};
use crate::store::Store;
use gateway::GatewayRouter;
use inference::InferenceRouter;
use media::MediaStore;
use protocol::{IdentityId, InboundMessage, PersonId, ProfileId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct Mind {
    pub(super) event_rx: mpsc::Receiver<WakeEvent>,
    pub(super) event_tx: mpsc::Sender<WakeEvent>,
    pub(super) registry: ActionRegistry,
    pub(super) state: StateHandle,
    pub(super) store: Arc<dyn Store>,
    pub(super) media_store: Option<Arc<MediaStore>>,
    pub(super) router: Arc<InferenceRouter>,
    pub(super) gateway: Arc<GatewayRouter>,
    pub(super) max_turns: usize,
    pub(super) max_action_attempts: usize,
    pub(super) escalate_after: usize,
}

impl Mind {
    pub fn new(
        event_rx: mpsc::Receiver<WakeEvent>,
        event_tx: mpsc::Sender<WakeEvent>,
        state: StateHandle,
        store: Arc<dyn Store>,
        media_store: Option<Arc<MediaStore>>,
        router: Arc<InferenceRouter>,
        gateway: Arc<GatewayRouter>,
        max_concurrency: usize,
        max_turns: usize,
        max_action_attempts: usize,
        escalate_after: usize,
    ) -> Self {
        Self {
            event_rx,
            event_tx,
            registry: ActionRegistry::new(max_concurrency),
            state,
            store,
            media_store,
            router,
            gateway,
            max_turns,
            max_action_attempts,
            escalate_after,
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
        if msg.identity.is_some() && msg.profile.is_some() {
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
            match self
                .resolve_or_create_identity_context(msg, Authority::Owner, Some(owner_id.clone()))
                .await
            {
                Some(ctx) => {
                    msg.identity = Some(ctx.identity.id);
                    msg.profile = Some(ctx.profile.id);
                    msg.person = ctx.person.map(|person| person.id).or(Some(owner_id));
                }
                None => {
                    let _ = self.store.touch_person(&owner_id).await;
                    msg.person = Some(owner_id);
                }
            }
        } else {
            let resolved = self
                .resolve_or_create_identity_context(msg, Authority::Owner, None)
                .await;
            if let Some(ctx) = resolved {
                let person_id = ctx.person.map(|person| person.id);
                if let Some(ref id) = person_id {
                    info!(person = %id.0, "created owner from first relay contact");
                }
                msg.identity = Some(ctx.identity.id);
                msg.profile = Some(ctx.profile.id);
                msg.person = person_id;
            }
        }
    }

    async fn resolve_gateway_person(&self, msg: &mut InboundMessage) {
        let authority = if self.find_owner().is_none() {
            Authority::Owner
        } else {
            Authority::Default
        };
        if let Some(ctx) = self
            .resolve_or_create_identity_context(msg, authority, None)
            .await
        {
            msg.identity = Some(ctx.identity.id);
            msg.profile = Some(ctx.profile.id);
            msg.person = ctx.person.map(|person| person.id);
        }
    }

    async fn resolve_or_create_identity_context(
        &self,
        msg: &InboundMessage,
        authority: Authority,
        attach_to: Option<PersonId>,
    ) -> Option<ResolvedActorIdentity> {
        match self
            .store
            .resolve_identity(&msg.gateway_id, &msg.external_id)
            .await
        {
            Ok(Some(ctx)) => {
                let _ = self.store.touch_identity(&ctx.identity.id).await;
                let _ = self.store.touch_profile(&ctx.profile.id).await;
                if let Some(person) = &ctx.person {
                    let _ = self.store.touch_person(&person.id).await;
                }
                return Some(ctx);
            }
            Ok(None) => {}
            Err(e) => warn!("failed to resolve identity: {e}"),
        }

        let now = chrono::Utc::now().timestamp();
        let identity = Identity {
            id: IdentityId(format!("identity-{}", nanoid::nanoid!())),
            gateway_id: msg.gateway_id.clone(),
            external_id: msg.external_id.clone(),
            display_name: None,
            metadata: None,
            created_at: now,
            last_seen_at: now,
        };
        let profile = Profile {
            id: ProfileId(format!("profile-{}", nanoid::nanoid!())),
            display_name: None,
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
            created_at: now,
            updated_at: now,
        };
        let created_person = attach_to.is_none();
        let person_id =
            attach_to.unwrap_or_else(|| PersonId(format!("person-{}", nanoid::nanoid!())));
        if created_person {
            let person = Person {
                id: person_id.clone(),
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
        }
        if let Err(e) = self.store.add_identity(&identity).await {
            warn!("failed to create identity: {e}");
            return None;
        }
        if let Err(e) = self.store.add_profile(&profile).await {
            warn!("failed to create profile: {e}");
            return None;
        }
        let evidence = serde_json::json!({
            "reason": "first_seen_gateway_identity",
            "gateway_id": msg.gateway_id,
            "external_id": msg.external_id,
        });
        if let Err(e) = self
            .store
            .link_identity_to_profile(&identity.id, &profile.id, 1.0, Some(&evidence))
            .await
        {
            warn!("failed to link identity to profile: {e}");
            return None;
        }
        let link = match self
            .store
            .attach_profile_to_person(
                &profile.id,
                &person_id,
                PersonProfileStatus::Verified,
                1.0,
                Some(&evidence),
            )
            .await
        {
            Ok(link) => link,
            Err(e) => {
                warn!("failed to attach profile to person: {e}");
                return None;
            }
        };
        if created_person || authority == Authority::Owner {
            let mut rel = Relationship::default();
            rel.authority = authority;
            self.state
                .shared
                .actor
                .write()
                .unwrap()
                .bonds
                .entry(person_id.clone())
                .or_insert(rel);
        }
        let person = self.store.get_person(&person_id).await.ok().flatten();
        Some(ResolvedActorIdentity {
            identity,
            profile,
            person,
            profile_person_link: Some(link),
        })
    }

    fn find_owner(&self) -> Option<PersonId> {
        let actor = self.state.read_state();
        actor
            .bonds
            .iter()
            .find(|(_, rel)| rel.authority == Authority::Owner)
            .map(|(id, _)| id.clone())
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
