use super::*;
use crate::request::SamplingConfig;
use crate::{
    ChatRequest, ChatResponse, ChatStream, InferenceProtocol, OpenAiCompatibleBridge, Retry,
};
use async_trait::async_trait;
use std::sync::Arc;

struct TestProvider;
struct FailingEmbeddingProvider;
struct SuccessfulEmbeddingProvider;

#[async_trait]
impl OpenAiCompatibleBridge for TestProvider {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("not used")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        anyhow::bail!("not used")
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for FailingEmbeddingProvider {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("not used")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        anyhow::bail!("not used")
    }

    async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("embedding endpoint unavailable")
    }
}

#[async_trait]
impl OpenAiCompatibleBridge for SuccessfulEmbeddingProvider {
    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("not used")
    }

    async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
        anyhow::bail!("not used")
    }

    async fn embed(&self, model: &str, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        assert_eq!(model, "embed-fallback");
        assert_eq!(input, &["deploy budget"]);
        Ok(vec![vec![0.1, 0.2, 0.3]])
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

fn embedding_endpoint(model: &str, provider: Arc<dyn OpenAiCompatibleBridge>) -> InferenceEndpoint {
    InferenceEndpoint {
        protocol: InferenceProtocol::OpenAiCompatible(provider),
        model: model.into(),
        sampling: SamplingConfig::default(),
        capabilities: vec![Capability::Embedding],
        reasoning: Reasoning::Basic,
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
    assert_eq!(chain[0].id, "vision");
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

#[tokio::test]
async fn embed_tries_next_embedding_route_when_first_fails() {
    let router = InferenceRouterBuilder::new()
        .endpoint(embedding_endpoint(
            "embed-primary",
            Arc::new(FailingEmbeddingProvider),
        ))
        .endpoint(embedding_endpoint(
            "embed-fallback",
            Arc::new(SuccessfulEmbeddingProvider),
        ))
        .build()
        .unwrap();

    let embeddings = router.embed(&["deploy budget"]).await.unwrap();

    assert_eq!(embeddings, vec![vec![0.1, 0.2, 0.3]]);
    let response = router
        .embed_with_metadata(&["deploy budget"])
        .await
        .unwrap();
    assert_eq!(response.endpoint_id, "embed-fallback");
    assert_eq!(response.model, "embed-fallback");
    assert_eq!(response.embeddings, vec![vec![0.1, 0.2, 0.3]]);
}

#[test]
fn endpoint_with_id_preserves_config_identifier() {
    let router = InferenceRouterBuilder::new()
        .endpoint_with_id(
            "chat-primary",
            endpoint("gpt-chat", Reasoning::Basic, vec![Capability::Chat]),
        )
        .build()
        .unwrap();

    let resolved = router.resolve(&RouteContext::Mind);

    assert_eq!(resolved.id, "chat-primary");
    assert_eq!(resolved.model, "gpt-chat");
}

#[tokio::test]
async fn embed_forwards_through_retry_wrapper() {
    let provider = Arc::new(Retry::new(SuccessfulEmbeddingProvider, 1));
    let router = InferenceRouterBuilder::new()
        .endpoint_with_id(
            "vpc-embedding",
            embedding_endpoint("embed-fallback", provider),
        )
        .build()
        .unwrap();

    let response = router
        .embed_with_metadata(&["deploy budget"])
        .await
        .unwrap();

    assert_eq!(response.endpoint_id, "vpc-embedding");
    assert_eq!(response.model, "embed-fallback");
    assert_eq!(response.embeddings, vec![vec![0.1, 0.2, 0.3]]);
}
