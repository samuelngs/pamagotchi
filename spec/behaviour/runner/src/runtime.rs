use inference::{
    CodexOptions, CodexProvider, InferenceEndpoint, InferenceProtocol, InferenceRouter,
    InferenceRouterBuilder, Retry, SamplingConfig,
};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct RuntimeSpec {
    pub schema_version: u32,
    pub default_inference: RuntimeInference,
}

#[derive(Debug, Deserialize)]
pub struct RuntimeInference {
    pub id: String,
    pub kind: String,
    pub capabilities: Vec<inference::Capability>,
    #[serde(default)]
    pub options: Value,
}

pub struct RuntimeConfig {
    pub spec: RuntimeSpec,
    pub router: Arc<InferenceRouter>,
    pub model: String,
}

impl RuntimeConfig {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join("spec/runtime.yaml");
        let raw = std::fs::read_to_string(&path)?;
        let spec: RuntimeSpec = yaml_serde::from_str(&raw)?;
        if spec.schema_version != 1 {
            anyhow::bail!("unsupported runtime schema_version {}", spec.schema_version);
        }
        let (router, model) = build_router(&spec.default_inference)?;
        Ok(Self {
            spec,
            router: Arc::new(router),
            model,
        })
    }

    pub fn summary(&self) -> String {
        let caps = self
            .spec
            .default_inference
            .capabilities
            .iter()
            .map(|cap| format!("{cap:?}").to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} kind={} model={} capabilities=[{}]",
            self.spec.default_inference.id, self.spec.default_inference.kind, self.model, caps
        )
    }
}

fn build_router(inference: &RuntimeInference) -> anyhow::Result<(InferenceRouter, String)> {
    match inference.kind.as_str() {
        "codex" => {
            let options: CodexOptions = serde_json::from_value(inference.options.clone())?;
            let model = options.model.clone();
            let provider = CodexProvider::new(options);
            let retry = Retry::new(provider, 1);
            let router = InferenceRouterBuilder::new()
                .endpoint(InferenceEndpoint {
                    protocol: InferenceProtocol::CodexAppServer(Arc::new(retry)),
                    model: model.clone(),
                    sampling: SamplingConfig::default(),
                    capabilities: inference.capabilities.clone(),
                    reasoning: inference::Reasoning::Basic,
                })
                .build()?;
            Ok((router, model))
        }
        other => anyhow::bail!("unsupported behaviour spec inference kind: {other}"),
    }
}
