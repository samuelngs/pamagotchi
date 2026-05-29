use super::*;

pub(super) fn build_inference_router(config: &Config) -> anyhow::Result<InferenceRouter> {
    let mut builder = InferenceRouterBuilder::new();

    for entry in &config.inference {
        let (provider, model, sampling) = build_provider(entry)?;
        builder = builder.endpoint_with_id(
            entry.id.clone(),
            InferenceEndpoint {
                protocol: provider,
                model,
                sampling,
                capabilities: entry.capabilities.clone(),
                reasoning: entry.reasoning,
            },
        );
    }

    builder.build()
}

fn build_provider(
    entry: &InferenceEntry,
) -> anyhow::Result<(InferenceProtocol, String, SamplingConfig)> {
    match &entry.provider {
        ProviderConfig::OpenAi(opts) => {
            let base_url = opts
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let api_key = match opts.api_key.as_deref() {
                Some(key) => key,
                None if base_url.contains("api.openai.com") => {
                    anyhow::bail!("api_key required for {base_url}");
                }
                None => "",
            };
            let provider = OpenAiProvider::new(base_url, api_key)
                .with_tool_choice_required(opts.tool_choice_required);
            let retry = Retry::new(provider, entry.max_retries)
                .with_base_delay(std::time::Duration::from_millis(entry.retry_delay_ms));
            let sampling = SamplingConfig {
                temperature: opts.temperature,
                top_p: opts.top_p,
                top_k: opts.top_k,
                min_p: opts.min_p,
            };
            Ok((
                InferenceProtocol::OpenAiCompatible(Arc::new(retry)),
                opts.model.clone(),
                sampling,
            ))
        }
        ProviderConfig::Codex(opts) => {
            let provider = CodexProvider::new(opts.clone());
            let retry = Retry::new(provider, entry.max_retries)
                .with_base_delay(std::time::Duration::from_millis(entry.retry_delay_ms));
            Ok((
                InferenceProtocol::CodexAppServer(Arc::new(retry)),
                opts.model.clone(),
                SamplingConfig::default(),
            ))
        }
    }
}
