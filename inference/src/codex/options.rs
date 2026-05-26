use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexOptions {
    #[serde(default = "default_codex_model")]
    pub model: String,

    #[serde(default = "default_codex_command")]
    pub command: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,

    #[serde(
        default = "default_codex_profile_v2",
        skip_serializing_if = "Option::is_none"
    )]
    pub profile_v2: Option<String>,

    #[serde(
        default = "default_codex_sandbox",
        skip_serializing_if = "Option::is_none"
    )]
    pub sandbox: Option<String>,

    #[serde(default)]
    pub extra_args: Vec<String>,
}

fn default_codex_model() -> String {
    "gpt-5.3-codex-spark".into()
}

fn default_codex_command() -> String {
    "codex".into()
}

fn default_codex_profile_v2() -> Option<String> {
    Some("pamagotchi".into())
}

fn default_codex_sandbox() -> Option<String> {
    Some("read-only".into())
}
