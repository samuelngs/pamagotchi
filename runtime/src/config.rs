use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    #[serde(default)]
    pub log: LogConfig,

    #[serde(default)]
    pub actors: Vec<ActorEntry>,
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

#[derive(Serialize, Deserialize)]
pub struct ActorEntry {
    pub id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderConfig>,

    #[serde(default = "default_max_turns")]
    pub max_turns: usize,

    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<PlatformEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct ProviderConfig {
    pub chat: ProviderEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<ProviderEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ProviderEntry {
    #[serde(rename = "openai")]
    OpenAi {
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        temperature: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        top_p: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        top_k: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_p: Option<f32>,
    },
}

impl ProviderEntry {
    pub fn model(&self) -> &str {
        match self {
            ProviderEntry::OpenAi { model, .. } => model,
        }
    }

    pub fn sampling(&self) -> actor::llm::SamplingConfig {
        match self {
            ProviderEntry::OpenAi {
                temperature,
                top_p,
                top_k,
                min_p,
                ..
            } => actor::llm::SamplingConfig {
                temperature: *temperature,
                top_p: *top_p,
                top_k: *top_k,
                min_p: *min_p,
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PlatformEntry {
    #[serde(rename = "whatsapp")]
    WhatsApp {},
}

fn default_data_dir() -> String {
    "~/.pamagotchi/data".into()
}

fn default_log_level() -> String {
    "info".into()
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

pub fn generate_id() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            log: LogConfig::default(),
            actors: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let expanded = expand_env_vars(&raw);
        let config: Config =
            serde_yaml::from_str(&expanded).context("failed to parse config yaml")?;
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
        let yaml = serde_yaml::to_string(self).context("failed to serialize config")?;
        std::fs::write(path, yaml).context("failed to write config")?;
        Ok(())
    }

    pub fn data_dir(&self) -> PathBuf {
        expand_tilde(&self.data_dir)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let mut ids = HashSet::new();

        for actor in &self.actors {
            if !ids.insert(&actor.id) {
                bail!("duplicate actor id: {}", actor.id);
            }
            if actor.id.len() != 64 || !actor.id.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("actor id must be 64 hex characters, got: {}", actor.id);
            }
        }
        Ok(())
    }

    pub fn default_path() -> PathBuf {
        expand_tilde("~/.pamagotchi/config.yml")
    }
}

impl ActorEntry {
    pub fn actor_data_dir(&self, base: &Path) -> PathBuf {
        base.join(&self.id)
    }

    pub fn store_path(&self, base: &Path) -> PathBuf {
        self.actor_data_dir(base).join("store.db")
    }
}

impl PlatformEntry {
    pub fn platform_id(&self) -> &str {
        match self {
            PlatformEntry::WhatsApp { .. } => "whatsapp",
        }
    }

    pub fn db_path(&self, actor_data_dir: &Path) -> PathBuf {
        actor_data_dir.join(format!("{}.db", self.platform_id()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
actors:
  - id: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    provider:
      chat:
        kind: openai
        model: gpt-4o
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.actors.len(), 1);
        assert_eq!(config.actors[0].max_turns, 5);
        let provider = config.actors[0].provider.as_ref().unwrap();
        assert_eq!(provider.chat.model(), "gpt-4o");
        assert!(provider.embedding.is_none());
        assert!(config.actors[0].platforms.is_empty());
        config.validate().unwrap();
    }

    #[test]
    fn parse_full_config() {
        let yaml = r#"
data_dir: /tmp/pamagotchi-test

log:
  level: debug

actors:
  - id: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    max_turns: 10
    max_concurrency: 3
    provider:
      chat:
        kind: openai
        base_url: https://api.openai.com/v1
        model: gpt-4o
      embedding:
        kind: openai
        model: text-embedding-3-small
    platforms:
      - kind: whatsapp
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        config.validate().unwrap();

        let actor = &config.actors[0];
        assert_eq!(actor.max_turns, 10);
        assert_eq!(actor.max_concurrency, 3);
        assert!(actor.provider.as_ref().unwrap().embedding.is_some());
        assert_eq!(actor.platforms.len(), 1);

        let data_dir = config.data_dir();
        assert_eq!(data_dir, PathBuf::from("/tmp/pamagotchi-test"));

        let actor_dir = actor.actor_data_dir(&data_dir);
        assert_eq!(
            actor_dir,
            PathBuf::from("/tmp/pamagotchi-test/a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2")
        );

        let wa_path = actor.platforms[0].db_path(&actor_dir);
        assert!(wa_path.ends_with("whatsapp.db"));
    }

    #[test]
    fn reject_duplicate_ids() {
        let yaml = r#"
actors:
  - id: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    provider:
      chat:
        kind: openai
        model: gpt-4o
  - id: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    provider:
      chat:
        kind: openai
        model: gpt-4o
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_bad_id() {
        let yaml = r#"
actors:
  - id: "tooshort"
    provider:
      chat:
        kind: openai
        model: gpt-4o
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
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
    fn platform_db_path() {
        let entry = PlatformEntry::WhatsApp {};
        let actor_dir = PathBuf::from("/data/abc123");
        assert_eq!(
            entry.db_path(&actor_dir),
            PathBuf::from("/data/abc123/whatsapp.db")
        );
    }

    #[test]
    fn generate_id_is_64_hex() {
        let id = generate_id();
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn roundtrip_serialize() {
        let mut config = Config::default();
        let id = generate_id();
        config.actors.push(ActorEntry {
            id: id.clone(),
            provider: None,
            max_turns: 5,
            max_concurrency: 5,
            platforms: vec![],
        });
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.actors.len(), 1);
        assert_eq!(parsed.actors[0].id, id);
    }
}
