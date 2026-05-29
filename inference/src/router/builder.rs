use super::router::InferenceRouter;
use super::types::{Capability, InferenceEndpoint, Reasoning, ResolvedRoute};
use std::collections::BTreeMap;

pub struct InferenceRouterBuilder {
    endpoints: Vec<ConfiguredEndpoint>,
}

struct ConfiguredEndpoint {
    id: String,
    endpoint: InferenceEndpoint,
}

impl InferenceRouterBuilder {
    pub fn new() -> Self {
        Self {
            endpoints: Vec::new(),
        }
    }

    pub fn endpoint(mut self, endpoint: InferenceEndpoint) -> Self {
        let id = endpoint.model.clone();
        self.endpoints.push(ConfiguredEndpoint { id, endpoint });
        self
    }

    pub fn endpoint_with_id(mut self, id: impl Into<String>, endpoint: InferenceEndpoint) -> Self {
        self.endpoints.push(ConfiguredEndpoint {
            id: id.into(),
            endpoint,
        });
        self
    }

    pub fn build(self) -> anyhow::Result<InferenceRouter> {
        if self.endpoints.is_empty() {
            anyhow::bail!("no inference endpoints configured");
        }

        let mut chat_map: BTreeMap<u8, Vec<ResolvedRoute>> = BTreeMap::new();
        let mut embedding = Vec::new();

        for configured in self.endpoints {
            let ep = configured.endpoint;
            let route = ResolvedRoute {
                id: configured.id,
                protocol: ep.protocol.clone(),
                model: ep.model.clone(),
                sampling: ep.sampling.clone(),
                capabilities: ep.capabilities.clone(),
            };

            if ep.capabilities.contains(&Capability::Embedding) {
                embedding.push(route);
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
