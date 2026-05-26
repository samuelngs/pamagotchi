use super::router::InferenceRouter;
use super::types::{Capability, InferenceEndpoint, Reasoning, ResolvedRoute};
use std::collections::BTreeMap;

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

        let mut chat_map: BTreeMap<u8, Vec<ResolvedRoute>> = BTreeMap::new();
        let mut embedding = Vec::new();

        for ep in self.endpoints {
            let route = ResolvedRoute {
                protocol: ep.protocol.clone(),
                model: ep.model.clone(),
                sampling: ep.sampling.clone(),
                capabilities: ep.capabilities.clone(),
            };

            if ep.capabilities.contains(&Capability::Embedding) {
                embedding.push(ResolvedRoute {
                    protocol: ep.protocol,
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
