use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodexEffort {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl CodexEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

    /// Codex app-server `turn/start.effort` override.
    ///
    /// This is distinct from the inference router `Reasoning` tier, which is
    /// only used internally to choose an endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<CodexEffort>,

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
