use super::*;
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::SharedState;
use crate::identity::{
    Group, GroupContext, Identity, Person, PersonProfileStatus, Profile, Relation, RelationSource,
    RelationStatus, SocialRelation,
};
use crate::state::{
    BehaviorDirective, CoreTraits, Delta, DirectiveScope, GrowthConfig, RelationshipSignalUpdate,
};
use crate::store::{
    ActionMessageRecord, ActionRunRecord, ActionTurnRecord, ChannelRecord, GatewayRecord,
    IntentRecord, Memory, MemoryKind, MemorySource, MemorySubject, MemoryType, MessageRole,
    OutboundDeliveryRecord, PrivacyCategory, SqliteStore, StoredMessage, Thought, ThoughtKind,
    ToolCallRecord,
};
use async_trait::async_trait;
use gateway::GatewayRouter;
use inference::{
    Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
    InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    Usage,
};
use protocol::{GroupId, IdentityId, MemoryId, PersonId, ProfileId};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

struct NoopBridge;

#[async_trait]
impl OpenAiCompatibleBridge for NoopBridge {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            message: inference::AssistantMessage {
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
        anyhow::bail!("noop bridge is not used by prompt tests")
    }
}

fn test_router() -> inference::InferenceRouter {
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

mod group_directive_tests;
mod identity_context_tests;
mod identity_tests;
mod outreach_prompt_tests;
mod review_prompt_tests;
