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

mod replay_tests;
mod scheduler_consolidation_tests;
mod scheduler_intent_tests;
mod scheduler_message_tests;
