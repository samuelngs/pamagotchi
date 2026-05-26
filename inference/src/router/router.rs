use super::types::{
    Capability, Reasoning, ResolvedInference, ResolvedRoute, RouteContext,
    has_required_capabilities, resolved_from_routes,
};

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
        let route = self
            .embedding
            .first()
            .ok_or_else(|| anyhow::anyhow!("no embedding endpoint configured"))?;
        route.protocol.embed(&route.model, input).await
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
