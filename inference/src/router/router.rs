use super::types::{
    Capability, EmbeddingResponse, Reasoning, ResolvedInference, ResolvedRoute, RouteContext,
    has_required_capabilities, resolved_from_routes,
};
use tracing::warn;

pub struct InferenceRouter {
    pub(super) chat: Vec<(Reasoning, Vec<ResolvedRoute>)>,
    pub(super) embedding: Vec<ResolvedRoute>,
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
            protocol: route.protocol.clone(),
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
        Ok(self.embed_with_metadata(input).await?.embeddings)
    }

    pub async fn embed_with_metadata(&self, input: &[&str]) -> anyhow::Result<EmbeddingResponse> {
        if self.embedding.is_empty() {
            anyhow::bail!("no embedding endpoint configured");
        }

        let mut last_error = None;
        for route in &self.embedding {
            match route.protocol.embed(&route.model, input).await {
                Ok(embeddings) => {
                    return Ok(EmbeddingResponse {
                        model: route.model.clone(),
                        embeddings,
                    });
                }
                Err(error) => {
                    warn!(
                        %error,
                        model = %route.model,
                        "embedding endpoint failed"
                    );
                    last_error = Some(error);
                }
            }
        }

        let message = last_error.map_or_else(
            || "unknown embedding failure".to_string(),
            |error| error.to_string(),
        );
        anyhow::bail!("all embedding endpoints failed: {message}")
    }

    fn find_chat(&self, level: Reasoning) -> Option<&ResolvedRoute> {
        self.find_chat_chain(level).first()
    }

    fn find_chat_chain(&self, level: Reasoning) -> &[ResolvedRoute] {
        let target = level.ordinal();
        if let Some((_, chain)) = self.chat.iter().find(|(r, _)| r.ordinal() == target) {
            return chain;
        }
        if let Some((_, chain)) = self.chat.iter().find(|(r, _)| r.ordinal() > target) {
            return chain;
        }
        self.chat.last().map(|(_, c)| c.as_slice()).unwrap_or(&[])
    }
}
