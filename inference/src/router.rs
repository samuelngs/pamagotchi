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
}

pub struct InferenceRouter {
    chat: Vec<(Reasoning, Vec<ResolvedRoute>)>,
    embedding: Vec<ResolvedRoute>,
}

pub struct ResolvedInference {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub sampling: SamplingConfig,
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
        }
    }

    pub fn resolve_chain(&self, ctx: &RouteContext) -> Vec<ResolvedInference> {
        let chain: &[ResolvedRoute] = match ctx {
            RouteContext::Mind => self.chat.first().map(|(_, c)| c.as_slice()).unwrap_or(&[]),
            RouteContext::Embedding => &self.embedding,
            RouteContext::Action(level) => self.find_chat_chain(*level),
        };
        if chain.is_empty() {
            return vec![self.resolve(ctx)];
        }
        chain
            .iter()
            .map(|r| ResolvedInference {
                provider: r.provider.clone(),
                model: r.model.clone(),
                sampling: r.sampling.clone(),
            })
            .collect()
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
            };

            if ep.capabilities.contains(&Capability::Embedding) {
                embedding.push(ResolvedRoute {
                    provider: ep.provider,
                    model: ep.model,
                    sampling: ep.sampling,
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
