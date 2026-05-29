use protocol::{ChannelId, ChannelKind, GatewayId, ProfileId, SpaceId, SpaceKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayRecord {
    pub id: GatewayId,
    pub kind: String,
    pub display_name: Option<String>,
    pub metadata: Value,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpaceRecord {
    pub id: SpaceId,
    pub gateway: GatewayId,
    pub external_id: String,
    pub kind: SpaceKind,
    pub display_name: Option<String>,
    pub metadata: Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelRecord {
    pub id: ChannelId,
    pub gateway: GatewayId,
    pub external_id: String,
    pub kind: ChannelKind,
    pub space: Option<SpaceId>,
    pub parent: Option<ChannelId>,
    pub display_name: Option<String>,
    pub metadata: Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelFilter {
    pub gateway: Option<GatewayId>,
    pub kind: Option<ChannelKind>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelMembership {
    pub channel: ChannelId,
    pub profile: ProfileId,
    pub role: Option<String>,
    pub status: ChannelMembershipStatus,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelMembershipStatus {
    Observed,
    Active,
    Left,
    Blocked,
}

impl ChannelMembershipStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Active => "active",
            Self::Left => "left",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "observed" => Some(Self::Observed),
            "active" => Some(Self::Active),
            "left" => Some(Self::Left),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}
