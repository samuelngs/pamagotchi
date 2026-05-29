use super::*;
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::SessionKind;
use crate::identity::{Identity, Person, PersonProfileStatus, Profile};
use crate::state::{ActorState, GrowthConfig, RelationshipStanding};
use crate::store::{
    Memory, MemoryKind, MemorySource, MemorySubject, MemorySubjectType, SqliteStore, Store,
};
use async_trait::async_trait;
use gateway::GatewayRouter;
use inference::{
    AssistantMessage, Capability, ChatRequest, ChatResponse, ChatStream, FinishReason,
    InferenceEndpoint, InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge,
    Reasoning, SamplingConfig, Usage,
};
use protocol::{ConversationId, IdentityId, InboundMessage, MemoryId, ProfileId};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

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
        anyhow::bail!("noop bridge is not used by identity tool tests")
    }
}

fn person(id: &str) -> Person {
    Person {
        id: PersonId(id.into()),
        name: Some(id.into()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
    }
}

fn test_context(store: Arc<SqliteStore>, claimant: PersonId) -> SessionContext {
    test_context_with_relationships(store, claimant, Vec::new())
}

fn test_context_with_relationships(
    store: Arc<SqliteStore>,
    claimant: PersonId,
    relationships: Vec<(PersonId, RelationshipStanding)>,
) -> SessionContext {
    let (_inject_tx, inject_rx) = mpsc::channel(1);
    let (delta_tx, _delta_rx) = mpsc::channel(1);
    let mut actor = ActorState::new(Default::default());
    for (person, relationship_standing) in relationships {
        actor.set_relationship_config(&person, Some(relationship_standing));
    }
    let shared = Arc::new(SharedState {
        actor: RwLock::new(actor),
        config: RwLock::new(GrowthConfig::default()),
    });
    let router = InferenceRouterBuilder::new()
        .endpoint(InferenceEndpoint {
            protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
            model: "noop".into(),
            sampling: SamplingConfig::default(),
            capabilities: vec![Capability::Chat],
            reasoning: Reasoning::Basic,
        })
        .build()
        .unwrap();
    let message = InboundMessage {
        message_id: "msg-1".into(),
        gateway_id: "relay".into(),
        sender_external_id: "claimant".into(),
        sender_display_name: Some("Claimant".into()),
        reply_external_id: "claimant".into(),
        conversation: ConversationId("relay:claimant".into()),
        group: None,
        identity: None,
        profile: None,
        person: Some(claimant),
        content: "i am person-a from discord".into(),
        attachments: vec![],
        timestamp: 1000,
        metadata: serde_json::Value::Null,
    };

    SessionContext {
        action_id: ActionId("identity-tool-test".into()),
        kind: SessionKind::Action(ActionKind::Respond),
        messages: vec![message],
        conversation: Some(ConversationId("relay:claimant".into())),
        relationship_standing: RelationshipStanding::Default,
        style_directive: None,
        cancelled_note: None,
        concurrent_summaries: vec![],
        state: StateHandle::new(shared, delta_tx),
        store,
        media_store: None,
        router: Arc::new(router),
        endpoints: vec![],
        reasoning: Reasoning::Basic,
        inject_rx,
        progress: Arc::new(RwLock::new(RunningState::new())),
        max_turns: 1,
        max_action_attempts: 1,
        escalate_after: 1,
        gateway: Arc::new(GatewayRouter::new()),
        typing: Arc::new(RwLock::new(Default::default())),
        metrics: Arc::new(crate::core::ActorMetrics::default()),
        session_start: std::time::Instant::now(),
    }
}

async fn attach_reachable_identity_to_person(store: &SqliteStore, person: &PersonId) {
    let identity = Identity {
        id: IdentityId(format!("identity-{}", person.0)),
        gateway_id: "relay".into(),
        external_id: format!("{}-external", person.0),
        display_name: Some(person.0.clone()),
        metadata: None,
        created_at: 1000,
        last_seen_at: 1000,
    };
    let profile = Profile {
        id: ProfileId(format!("profile-{}", person.0)),
        display_name: Some(person.0.clone()),
        summary: None,
        comm_style: None,
        first_seen: 1000,
        last_seen: 1000,
        created_at: 1000,
        updated_at: 1000,
    };
    store.add_identity(&identity).await.unwrap();
    store.add_profile(&profile).await.unwrap();
    store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, None)
        .await
        .unwrap();
    store
        .attach_profile_to_person(
            &profile.id,
            person,
            PersonProfileStatus::Verified,
            1.0,
            None,
        )
        .await
        .unwrap();
}

mod claim_tests;
mod memory_demotion_tests;
mod verification_tests;
