use protocol::GatewaySetupInstructions;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug)]
pub struct DiscordConfig {
    pub bot_token: String,
    pub allowed_channel_ids: HashSet<u64>,
    pub ignore_bots: bool,
}

impl DiscordConfig {
    pub fn from_vars(vars: &BTreeMap<String, Value>) -> anyhow::Result<Self> {
        let bot_token = token_from_vars(vars)?;
        let allowed_channel_ids = string_array_var(vars, "allowed_channel_ids")?
            .into_iter()
            .map(|id| {
                id.parse::<u64>()
                    .map_err(|_| anyhow::anyhow!("invalid Discord channel id: {id}"))
            })
            .collect::<anyhow::Result<HashSet<_>>>()?;
        let ignore_bots = vars
            .get("ignore_bots")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        Ok(Self {
            bot_token,
            allowed_channel_ids,
            ignore_bots,
        })
    }

    pub fn allows_channel(&self, channel_id: u64) -> bool {
        self.allowed_channel_ids.is_empty() || self.allowed_channel_ids.contains(&channel_id)
    }
}

fn token_from_vars(vars: &BTreeMap<String, Value>) -> anyhow::Result<String> {
    vars.get("bot_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("Discord bot token is required in gateway vars.bot_token"))
}

fn string_array_var(vars: &BTreeMap<String, Value>, key: &str) -> anyhow::Result<Vec<String>> {
    let Some(value) = vars.get(key) else {
        return Ok(vec![]);
    };
    let Some(values) = value.as_array() else {
        anyhow::bail!("{key} must be an array of strings");
    };

    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("{key} must contain only non-empty strings"))
        })
        .collect()
}

pub fn setup_instructions() -> GatewaySetupInstructions {
    GatewaySetupInstructions::Text {
        title: "Connect Discord".into(),
        body: "Create a Discord bot, invite it to the server, enable the Message Content intent in the developer portal, then set bot_token in this gateway's vars.".into(),
    }
}

pub fn is_missing_bot_token_error(error: &anyhow::Error) -> bool {
    error.to_string() == "Discord bot token is required in gateway vars.bot_token"
}
