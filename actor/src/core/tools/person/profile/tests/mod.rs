use super::{get, identity_lookup_reason, mask_external_id};
use crate::core::action::{ActionId, ActionKind, RunningState};
use crate::core::handle::{SharedState, StateHandle};
use crate::core::tools::{SessionContext, SessionKind};
use crate::identity::{Identity, Person, PersonProfileStatus, Profile, ProfileIdentityStatus};
use crate::state::{ActorState, GrowthConfig, RelationshipStanding};
use crate::store::{SqliteStore, Store};
use async_trait::async_trait;
use gateway::GatewayRouter;
use inference::{
    Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
    InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    Usage,
};
use protocol::{ConversationId, IdentityId, InboundMessage, PersonId, ProfileId};
use serde_json::json;
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
        anyhow::bail!("noop bridge is not used by get_person tests")
    }
}

mod lookup_tests;
mod masking_tests;
