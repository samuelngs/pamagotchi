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
    pub(super) fn ordinal(self) -> u8 {
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

pub(super) struct ResolvedRoute {
    pub(super) provider: Arc<dyn Provider>,
    pub(super) model: String,
    pub(super) sampling: SamplingConfig,
    pub(super) capabilities: Vec<Capability>,
}

#[derive(Clone)]
pub struct ResolvedInference {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub sampling: SamplingConfig,
    pub capabilities: Vec<Capability>,
}

pub struct InferenceEndpoint {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub sampling: SamplingConfig,
    pub capabilities: Vec<Capability>,
    pub reasoning: Reasoning,
}

pub(super) fn has_required_capabilities(have: &[Capability], required: &[Capability]) -> bool {
    required.iter().all(|cap| have.contains(cap))
}

pub(super) fn resolved_from_routes(routes: Vec<&ResolvedRoute>) -> Vec<ResolvedInference> {
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
