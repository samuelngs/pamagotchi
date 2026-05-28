use anyhow::{Context, bail};
use inference::{Capability, CodexOptions, OpenAiOptions, Reasoning};
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

    #[serde(default)]
    pub retry: RetryConfig,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,

    #[serde(default = "default_escalate_after")]
    pub escalate_after: usize,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            escalate_after: default_escalate_after(),
        }
    }
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
    #[serde(rename = "codex")]
    Codex(CodexOptions),
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

fn default_max_attempts() -> usize {
    3
}

fn default_escalate_after() -> usize {
    1
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
            retry: RetryConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let expanded = expand_env_vars(&raw);
        let config: Config =
            yaml_serde::from_str(&expanded).context("failed to parse config yaml")?;
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
        let yaml = yaml_serde::to_string(self).context("failed to serialize config")?;
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
mod tests;
