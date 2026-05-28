use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{MediaAssetId, MediaKind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayConnectionState {
    NotConfigured,
    SetupRequired,
    Connecting,
    Connected,
    Disconnected,
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewaySetupInstructions {
    Text {
        title: String,
        body: String,
    },
    QrCode {
        title: String,
        body: String,
        code: String,
        rendered: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GatewayView {
    pub id: String,
    pub kind: String,

    #[serde(default)]
    pub vars: Value,

    pub connection_state: GatewayConnectionState,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_instructions: Option<GatewaySetupInstructions>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayKindView {
    pub kind: String,

    #[serde(default)]
    pub vars: Vec<GatewayVarSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayVarSpec {
    pub key: String,
    pub label: String,
    pub kind: GatewayVarKind,

    #[serde(default)]
    pub required: bool,

    #[serde(default)]
    pub secret: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayVarKind {
    String,
    Bool,
    StringList,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAssetView {
    pub id: MediaAssetId,
    pub kind: MediaKind,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionTopic {
    Chat,
    Gateways,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientRequest {
    Subscribe {
        topics: Vec<SubscriptionTopic>,
    },
    SendChatMessage {
        content: String,
    },
    CreateMediaAsset {
        request_id: String,
        kind: String,
        data_base64: String,
        #[serde(default)]
        mime: Option<String>,
        #[serde(default)]
        filename: Option<String>,
    },
    ListGateways {
        request_id: String,
    },
    ListAvailableGateways {
        request_id: String,
    },
    AddGateway {
        request_id: String,
        kind: String,
        #[serde(default)]
        vars: Value,
    },
    RemoveGateway {
        request_id: String,
        id: String,
    },
    RestartGateway {
        request_id: String,
        id: String,
    },
    UpdateGatewayVars {
        request_id: String,
        id: String,
        #[serde(default)]
        vars: Value,
    },
    GetDebugSnapshot {
        request_id: String,
        #[serde(default)]
        limit: Option<usize>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    ChatMessage {
        content: String,
        is_self: bool,
    },
    ComposingStarted,
    ComposingStopped,
    GatewayList {
        request_id: String,
        gateways: Vec<GatewayView>,
    },
    AvailableGatewayList {
        request_id: String,
        gateways: Vec<GatewayKindView>,
    },
    GatewayAdded {
        gateway: GatewayView,
    },
    GatewayRemoved {
        id: String,
    },
    GatewayUpdated {
        gateway: GatewayView,
    },
    GatewayConnectionStateChanged {
        id: String,
        state: GatewayConnectionState,
    },
    GatewaySetupInstructionsChanged {
        id: String,
        setup: Option<GatewaySetupInstructions>,
    },
    MediaAssetCreated {
        request_id: String,
        asset: MediaAssetView,
    },
    RequestOk {
        request_id: String,
    },
    RequestError {
        request_id: Option<String>,
        message: String,
    },
    DebugSnapshot {
        request_id: String,
        snapshot: Value,
    },
}

#[cfg(test)]
mod tests;
