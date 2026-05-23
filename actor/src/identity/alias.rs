use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alias {
    pub platform: Platform,
    pub platform_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Discord,
    Telegram,
    WhatsApp,
    Signal,
    Internal,
    Custom(String),
}

impl Platform {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::WhatsApp => "whatsapp",
            Self::Signal => "signal",
            Self::Internal => "internal",
            Self::Custom(s) => s,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "discord" => Self::Discord,
            "telegram" => Self::Telegram,
            "whatsapp" => Self::WhatsApp,
            "signal" => Self::Signal,
            "internal" => Self::Internal,
            other => Self::Custom(other.to_string()),
        }
    }
}
