use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct CodexOptions {
    pub model: String,

    #[serde(default = "default_codex_command")]
    pub command: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_v2: Option<String>,

    #[serde(
        default = "default_codex_sandbox",
        skip_serializing_if = "Option::is_none"
    )]
    pub sandbox: Option<String>,

    #[serde(
        default = "default_codex_approval_policy",
        skip_serializing_if = "Option::is_none"
    )]
    pub approval_policy: Option<String>,

    #[serde(default = "default_true")]
    pub ephemeral: bool,

    #[serde(default = "default_true")]
    pub skip_git_repo_check: bool,

    #[serde(default)]
    pub search: bool,

    #[serde(default)]
    pub ignore_user_config: bool,

    #[serde(default)]
    pub ignore_rules: bool,

    #[serde(default)]
    pub extra_args: Vec<String>,
}

fn default_codex_command() -> String {
    "codex".into()
}

fn default_true() -> bool {
    true
}

fn default_codex_sandbox() -> Option<String> {
    Some("read-only".into())
}

fn default_codex_approval_policy() -> Option<String> {
    Some("never".into())
}
