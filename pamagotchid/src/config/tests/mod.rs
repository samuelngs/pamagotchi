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
    let config: Config = yaml_serde::from_str(yaml).unwrap();
    config.validate().unwrap();
    assert_eq!(config.inference.len(), 1);
    let ProviderConfig::OpenAi(ref opts) = config.inference[0].provider else {
        panic!("expected openai provider");
    };
    assert_eq!(opts.model, "gpt-4o");
    assert_eq!(config.max_turns, 5);
}

#[test]
fn parse_codex_config() {
    let yaml = r#"
inference:
  - id: codex
    kind: codex
    capabilities: [chat]
    options: {}
"#;
    let config: Config = yaml_serde::from_str(yaml).unwrap();
    config.validate().unwrap();
    let ProviderConfig::Codex(ref opts) = config.inference[0].provider else {
        panic!("expected codex provider");
    };
    assert_eq!(opts.model, "gpt-5.3-codex-spark");
    assert_eq!(opts.command, "codex");
    assert_eq!(opts.profile_v2.as_deref(), Some("pamagotchi"));
    assert_eq!(opts.sandbox.as_deref(), Some("read-only"));
    assert!(opts.extra_args.is_empty());
}

#[test]
fn parse_codex_config_overrides_defaults() {
    let yaml = r#"
inference:
  - id: codex
    kind: codex
    capabilities: [chat]
    options:
      model: gpt-5.3-codex
      profile_v2: custom
"#;
    let config: Config = yaml_serde::from_str(yaml).unwrap();
    config.validate().unwrap();
    let ProviderConfig::Codex(ref opts) = config.inference[0].provider else {
        panic!("expected codex provider");
    };
    assert_eq!(opts.model, "gpt-5.3-codex");
    assert_eq!(opts.profile_v2.as_deref(), Some("custom"));
}

#[test]
fn reject_unknown_codex_option() {
    let yaml = r#"
inference:
  - id: codex
    kind: codex
    capabilities: [chat]
    options:
      model: gpt-5
      approval_policy: never
"#;
    let Err(err) = yaml_serde::from_str::<Config>(yaml) else {
        panic!("expected unknown codex option to fail");
    };
    assert!(err.to_string().contains("unknown field"));
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
    let config: Config = yaml_serde::from_str(yaml).unwrap();
    config.validate().unwrap();

    assert_eq!(config.inference.len(), 3);
    assert_eq!(config.max_turns, 10);
    assert_eq!(config.max_concurrency, 3);

    assert_eq!(config.inference[0].reasoning, Reasoning::Basic);
    assert_eq!(config.inference[1].reasoning, Reasoning::Advanced);
    let ProviderConfig::OpenAi(ref opts) = config.inference[0].provider else {
        panic!("expected openai provider");
    };
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
    let config: Config = yaml_serde::from_str(yaml).unwrap();
    assert!(config.validate().is_err());
}

#[test]
fn reject_empty_inference() {
    let yaml = r#"
inference: []
"#;
    let config: Config = yaml_serde::from_str(yaml).unwrap();
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
    let yaml = yaml_serde::to_string(&config).unwrap();
    let parsed: Config = yaml_serde::from_str(&yaml).unwrap();
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
    let config: Config = yaml_serde::from_str(yaml).unwrap();
    assert_eq!(config.inference[0].reasoning, Reasoning::Basic);
}
