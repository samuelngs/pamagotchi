use anyhow::{Context, bail};
use inference::{Capability, OpenAiOptions, Reasoning};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    #[serde(default)]
    pub log: LogConfig,

    #[serde(default)]
    pub inference: Vec<InferenceEntry>,

    #[serde(default = "default_max_turns")]
    pub max_turns: usize,

    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
}

#[derive(Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InferenceEntry {
    pub id: String,

    #[serde(default)]
    pub capabilities: Vec<Capability>,

    #[serde(default)]
    pub reasoning: Reasoning,

    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,

    #[serde(flatten)]
    pub provider: ProviderConfig,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "options")]
pub enum ProviderConfig {
    #[serde(rename = "openai")]
    OpenAi(OpenAiOptions),
}

fn default_data_dir() -> String {
    "~/.pamagotchi/data".into()
}

fn default_log_level() -> String {
    "info".into()
}

fn default_max_retries() -> u32 {
    3
}

fn default_retry_delay_ms() -> u64 {
    1000
}

fn default_max_turns() -> usize {
    5
}

fn default_max_concurrency() -> usize {
    5
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next();
            let mut var_name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                var_name.push(c);
            }
            if let Ok(val) = std::env::var(&var_name) {
                result.push_str(&val);
            }
        } else {
            result.push(c);
        }
    }
    result
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            log: LogConfig::default(),
            inference: Vec::new(),
            max_turns: default_max_turns(),
            max_concurrency: default_max_concurrency(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let expanded = expand_env_vars(&raw);
        let config: Config =
            serde_yml::from_str(&expanded).context("failed to parse config yaml")?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yml::to_string(self).context("failed to serialize config")?;
        std::fs::write(path, yaml).context("failed to write config")?;
        Ok(())
    }

    pub fn data_dir(&self) -> PathBuf {
        expand_tilde(&self.data_dir)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.inference.is_empty() {
            bail!("at least one inference entry required");
        }

        let mut ids = std::collections::HashSet::new();
        for entry in &self.inference {
            if !ids.insert(&entry.id) {
                bail!("duplicate inference id: {}", entry.id);
            }
        }

        Ok(())
    }

    pub fn store_path(&self) -> PathBuf {
        self.data_dir().join("store.db")
    }

    pub fn default_path() -> PathBuf {
        expand_tilde("~/.pamagotchi/config.yml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
inference:
  - id: default
    kind: openai
    capabilities: [chat]
    options:
      model: gpt-4o
"#;
        let config: Config = serde_yml::from_str(yaml).unwrap();
        config.validate().unwrap();
        assert_eq!(config.inference.len(), 1);
        let ProviderConfig::OpenAi(ref opts) = config.inference[0].provider;
        assert_eq!(opts.model, "gpt-4o");
        assert_eq!(config.max_turns, 5);
    }

    #[test]
    fn parse_full_config() {
        let yaml = r#"
data_dir: /tmp/pamagotchi-test

log:
  level: debug

inference:
  - id: flash
    kind: openai
    capabilities: [chat]
    reasoning: basic
    options:
      model: deepseek-v4-flash
      base_url: https://opencode.ai/zen/go/v1
      temperature: 0.7

  - id: smart
    kind: openai
    capabilities: [chat, vision]
    reasoning: advanced
    options:
      model: gpt-4o
      base_url: https://api.openai.com/v1

  - id: embedder
    kind: openai
    capabilities: [embedding]
    options:
      model: text-embedding-3-small

max_turns: 10
max_concurrency: 3
"#;
        let config: Config = serde_yml::from_str(yaml).unwrap();
        config.validate().unwrap();

        assert_eq!(config.inference.len(), 3);
        assert_eq!(config.max_turns, 10);
        assert_eq!(config.max_concurrency, 3);

        assert_eq!(config.inference[0].reasoning, Reasoning::Basic);
        assert_eq!(config.inference[1].reasoning, Reasoning::Advanced);
        let ProviderConfig::OpenAi(ref opts) = config.inference[0].provider;
        assert_eq!(opts.model, "deepseek-v4-flash");

        let data_dir = config.data_dir();
        assert_eq!(data_dir, PathBuf::from("/tmp/pamagotchi-test"));
    }

    #[test]
    fn reject_duplicate_inference_ids() {
        let yaml = r#"
inference:
  - id: default
    kind: openai
    options:
      model: gpt-4o
  - id: default
    kind: openai
    options:
      model: gpt-4o-mini
"#;
        let config: Config = serde_yml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_empty_inference() {
        let yaml = r#"
inference: []
"#;
        let config: Config = serde_yml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn env_var_expansion() {
        let home = std::env::var("HOME").unwrap();
        let input = "path: ${HOME}/data";
        let expanded = expand_env_vars(input);
        assert_eq!(expanded, format!("path: {home}/data"));
    }

    #[test]
    fn missing_env_var_expands_to_empty() {
        let input = "api_key: ${DEFINITELY_DOES_NOT_EXIST_12345}";
        let expanded = expand_env_vars(input);
        assert_eq!(expanded, "api_key: ");
    }

    #[test]
    fn roundtrip_serialize() {
        let config = Config {
            inference: vec![InferenceEntry {
                id: "default".into(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Standard,
                max_retries: 3,
                retry_delay_ms: 1000,
                provider: ProviderConfig::OpenAi(inference::OpenAiOptions {
                    model: "gpt-4o".into(),
                    base_url: None,
                    api_key: None,
                    temperature: None,
                    top_p: None,
                    top_k: None,
                    min_p: None,
                    tool_choice_required: true,
                }),
            }],
            ..Config::default()
        };
        let yaml = serde_yml::to_string(&config).unwrap();
        let parsed: Config = serde_yml::from_str(&yaml).unwrap();
        assert_eq!(parsed.inference.len(), 1);
        assert_eq!(parsed.inference[0].id, "default");
    }

    #[test]
    fn default_reasoning_is_basic() {
        let yaml = r#"
inference:
  - id: default
    kind: openai
    capabilities: [chat]
    options:
      model: gpt-4o
"#;
        let config: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(config.inference[0].reasoning, Reasoning::Basic);
    }
}
