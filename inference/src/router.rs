use crate::Provider;
use crate::request::SamplingConfig;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Chat,
    Embedding,
    Vision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reasoning {
    Basic,
    Standard,
    Advanced,
    Expert,
}

impl Default for Reasoning {
    fn default() -> Self {
        Self::Basic
    }
}

impl Reasoning {
    fn ordinal(self) -> u8 {
        match self {
            Self::Basic => 0,
            Self::Standard => 1,
            Self::Advanced => 2,
            Self::Expert => 3,
        }
    }

    pub fn escalate(self) -> Self {
        match self {
            Self::Basic => Self::Standard,
            Self::Standard => Self::Advanced,
            Self::Advanced => Self::Expert,
            Self::Expert => Self::Expert,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RouteContext {
    Mind,
    Embedding,
    Action(Reasoning),
}

struct ResolvedRoute {
    provider: Arc<dyn Provider>,
    model: String,
    sampling: SamplingConfig,
    capabilities: Vec<Capability>,
}

pub struct InferenceRouter {
    chat: Vec<(Reasoning, Vec<ResolvedRoute>)>,
    embedding: Vec<ResolvedRoute>,
}

#[derive(Clone)]
pub struct ResolvedInference {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub sampling: SamplingConfig,
    pub capabilities: Vec<Capability>,
}

impl InferenceRouter {
    pub fn resolve(&self, ctx: &RouteContext) -> ResolvedInference {
        let route = match ctx {
            RouteContext::Mind => self.chat.first().and_then(|(_, chain)| chain.first()),
            RouteContext::Embedding => self.embedding.first(),
            RouteContext::Action(level) => self.find_chat(*level),
        };
        let route = route
            .or_else(|| self.chat.first().and_then(|(_, c)| c.first()))
            .expect("router has no endpoints");
        ResolvedInference {
            provider: route.provider.clone(),
            model: route.model.clone(),
            sampling: route.sampling.clone(),
            capabilities: route.capabilities.clone(),
        }
    }

    pub fn resolve_chain(&self, ctx: &RouteContext) -> Vec<ResolvedInference> {
        self.resolve_chain_requiring(ctx, &[])
    }

    pub fn resolve_chain_requiring(
        &self,
        ctx: &RouteContext,
        required: &[Capability],
    ) -> Vec<ResolvedInference> {
        let target_ord = match ctx {
            RouteContext::Mind => Some(Reasoning::Basic.ordinal()),
            RouteContext::Action(level) => Some(level.ordinal()),
            RouteContext::Embedding => None,
        };
        let chain: &[ResolvedRoute] = match ctx {
            RouteContext::Mind => self.chat.first().map(|(_, c)| c.as_slice()).unwrap_or(&[]),
            RouteContext::Embedding => &self.embedding,
            RouteContext::Action(level) => self.find_chat_chain(*level),
        };
        if chain.is_empty() && required.is_empty() {
            return vec![self.resolve(ctx)];
        }

        let matches = chain
            .iter()
            .filter(|r| has_required_capabilities(&r.capabilities, required))
            .collect::<Vec<_>>();

        if !matches.is_empty() || required.is_empty() || target_ord.is_none() {
            return resolved_from_routes(matches);
        }

        for (_, chain) in self
            .chat
            .iter()
            .filter(|(reasoning, _)| reasoning.ordinal() >= target_ord.unwrap())
        {
            let matches = chain
                .iter()
                .filter(|r| has_required_capabilities(&r.capabilities, required))
                .collect::<Vec<_>>();
            if !matches.is_empty() {
                return resolved_from_routes(matches);
            }
        }

        Vec::new()
    }

    pub fn chat_supports(&self, required: &[Capability]) -> bool {
        self.chat
            .iter()
            .flat_map(|(_, chain)| chain.iter())
            .any(|route| has_required_capabilities(&route.capabilities, required))
    }

    pub fn has_embedding(&self) -> bool {
        !self.embedding.is_empty()
    }

    pub async fn embed(&self, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let route = self
            .embedding
            .first()
            .ok_or_else(|| anyhow::anyhow!("no embedding endpoint configured"))?;
        route.provider.embed(&route.model, input).await
    }

    fn find_chat(&self, level: Reasoning) -> Option<&ResolvedRoute> {
        self.find_chat_chain(level).first()
    }

    fn find_chat_chain(&self, level: Reasoning) -> &[ResolvedRoute] {
        let target = level.ordinal();
        if let Some((_, chain)) = self.chat.iter().find(|(r, _)| r.ordinal() == target) {
            return chain;
        }
        // nearest above
        if let Some((_, chain)) = self.chat.iter().find(|(r, _)| r.ordinal() > target) {
            return chain;
        }
        // highest available
        self.chat.last().map(|(_, c)| c.as_slice()).unwrap_or(&[])
    }
}

pub struct InferenceEndpoint {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub sampling: SamplingConfig,
    pub capabilities: Vec<Capability>,
    pub reasoning: Reasoning,
}

pub struct InferenceRouterBuilder {
    endpoints: Vec<InferenceEndpoint>,
}

impl InferenceRouterBuilder {
    pub fn new() -> Self {
        Self {
            endpoints: Vec::new(),
        }
    }

    pub fn endpoint(mut self, endpoint: InferenceEndpoint) -> Self {
        self.endpoints.push(endpoint);
        self
    }

    pub fn build(self) -> anyhow::Result<InferenceRouter> {
        if self.endpoints.is_empty() {
            anyhow::bail!("no inference endpoints configured");
        }

        let mut chat_map: std::collections::BTreeMap<u8, Vec<ResolvedRoute>> =
            std::collections::BTreeMap::new();
        let mut embedding = Vec::new();

        for ep in self.endpoints {
            let route = ResolvedRoute {
                provider: ep.provider.clone(),
                model: ep.model.clone(),
                sampling: ep.sampling.clone(),
                capabilities: ep.capabilities.clone(),
            };

            if ep.capabilities.contains(&Capability::Embedding) {
                embedding.push(ResolvedRoute {
                    provider: ep.provider,
                    model: ep.model,
                    sampling: ep.sampling,
                    capabilities: ep.capabilities,
                });
                continue;
            }

            if ep.capabilities.contains(&Capability::Chat) {
                chat_map
                    .entry(ep.reasoning.ordinal())
                    .or_default()
                    .push(route);
            }
        }

        let chat: Vec<(Reasoning, Vec<ResolvedRoute>)> = chat_map
            .into_iter()
            .map(|(ord, routes)| {
                let reasoning = match ord {
                    0 => Reasoning::Basic,
                    1 => Reasoning::Standard,
                    2 => Reasoning::Advanced,
                    _ => Reasoning::Expert,
                };
                (reasoning, routes)
            })
            .collect();

        if chat.is_empty() && embedding.is_empty() {
            anyhow::bail!("no usable endpoints (need at least one chat or embedding endpoint)");
        }

        Ok(InferenceRouter { chat, embedding })
    }
}

fn has_required_capabilities(have: &[Capability], required: &[Capability]) -> bool {
    required.iter().all(|cap| have.contains(cap))
}

fn resolved_from_routes(routes: Vec<&ResolvedRoute>) -> Vec<ResolvedInference> {
    routes
        .into_iter()
        .map(|r| ResolvedInference {
            provider: r.provider.clone(),
            model: r.model.clone(),
            sampling: r.sampling.clone(),
            capabilities: r.capabilities.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatRequest, ChatResponse, ChatStream};
    use async_trait::async_trait;

    struct TestProvider;

    #[async_trait]
    impl Provider for TestProvider {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("not used")
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("not used")
        }
    }

    fn endpoint(
        model: &str,
        reasoning: Reasoning,
        capabilities: Vec<Capability>,
    ) -> InferenceEndpoint {
        InferenceEndpoint {
            provider: Arc::new(TestProvider),
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
}
