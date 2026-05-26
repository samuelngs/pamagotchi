use super::*;
use crate::request::SamplingConfig;
use crate::{ChatRequest, ChatResponse, ChatStream, InferenceProtocol, OpenAiCompatibleBridge};
use async_trait::async_trait;
use std::sync::Arc;

struct TestProvider;

#[async_trait]
impl OpenAiCompatibleBridge for TestProvider {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("not used")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        anyhow::bail!("not used")
    }
}

fn endpoint(model: &str, reasoning: Reasoning, capabilities: Vec<Capability>) -> InferenceEndpoint {
    InferenceEndpoint {
        protocol: InferenceProtocol::OpenAiCompatible(Arc::new(TestProvider)),
        model: model.into(),
        sampling: SamplingConfig::default(),
        capabilities,
        reasoning,
    }
}

#[test]
fn resolve_chain_requiring_vision_escalates_to_capable_route() {
    let router = InferenceRouterBuilder::new()
        .endpoint(endpoint(
            "text",
            Reasoning::Standard,
            vec![Capability::Chat],
        ))
        .endpoint(endpoint(
            "vision",
            Reasoning::Advanced,
            vec![Capability::Chat, Capability::Vision],
        ))
        .build()
        .unwrap();

    let chain = router.resolve_chain_requiring(
        &RouteContext::Action(Reasoning::Standard),
        &[Capability::Vision],
    );

    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].model, "vision");
}

#[test]
fn resolve_chain_requiring_missing_capability_returns_empty() {
    let router = InferenceRouterBuilder::new()
        .endpoint(endpoint(
            "text",
            Reasoning::Standard,
            vec![Capability::Chat],
        ))
        .build()
        .unwrap();

    let chain = router.resolve_chain_requiring(
        &RouteContext::Action(Reasoning::Standard),
        &[Capability::Vision],
    );

    assert!(chain.is_empty());
}
