use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    RequestOk {
        request_id: String,
    },
    RequestError {
        request_id: Option<String>,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_subscribe_request_with_tagged_type() {
        let request = ClientRequest::Subscribe {
            topics: vec![SubscriptionTopic::Chat, SubscriptionTopic::Gateways],
        };

        let json = serde_json::to_value(request).unwrap();

        assert_eq!(json["type"], "Subscribe");
        assert_eq!(json["topics"][0], "Chat");
        assert_eq!(json["topics"][1], "Gateways");
    }

    #[test]
    fn serializes_gateway_list_event() {
        let event = ServerEvent::GatewayList {
            request_id: "req-1".into(),
            gateways: vec![GatewayView {
                id: "gw-1".into(),
                kind: "whatsapp".into(),
                vars: serde_json::json!({}),
                connection_state: GatewayConnectionState::SetupRequired,
                setup_instructions: Some(GatewaySetupInstructions::QrCode {
                    title: "Connect WhatsApp".into(),
                    body: "Scan this QR code from WhatsApp > Linked devices.".into(),
                    code: "qr-code".into(),
                    rendered: "qr-rendered".into(),
                }),
            }],
        };

        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["type"], "GatewayList");
        assert_eq!(json["request_id"], "req-1");
        assert_eq!(json["gateways"][0]["id"], "gw-1");
        assert_eq!(json["gateways"][0]["connection_state"], "SetupRequired");
        assert_eq!(
            json["gateways"][0]["setup_instructions"]["QrCode"]["code"],
            "qr-code"
        );
        assert_eq!(
            json["gateways"][0]["setup_instructions"]["QrCode"]["rendered"],
            "qr-rendered"
        );
    }

    #[test]
    fn deserializes_add_gateway_request() {
        let json = serde_json::json!({
            "type": "AddGateway",
            "request_id": "req-2",
            "kind": "discord",
            "vars": {
                "bot_token": "secret"
            }
        });

        let request: ClientRequest = serde_json::from_value(json).unwrap();

        assert_eq!(
            request,
            ClientRequest::AddGateway {
                request_id: "req-2".into(),
                kind: "discord".into(),
                vars: serde_json::json!({
                    "bot_token": "secret"
                }),
            }
        );
    }

    #[test]
    fn serializes_available_gateway_list_event() {
        let event = ServerEvent::AvailableGatewayList {
            request_id: "req-3".into(),
            gateways: vec![GatewayKindView {
                kind: "whatsapp".into(),
                vars: vec![],
            }],
        };

        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["type"], "AvailableGatewayList");
        assert_eq!(json["request_id"], "req-3");
        assert_eq!(json["gateways"][0]["kind"], "whatsapp");
        assert_eq!(json["gateways"][0]["vars"], serde_json::json!([]));
    }

    #[test]
    fn serializes_gateway_kind_var_specs() {
        let event = ServerEvent::AvailableGatewayList {
            request_id: "req-4".into(),
            gateways: vec![GatewayKindView {
                kind: "discord".into(),
                vars: vec![GatewayVarSpec {
                    key: "bot_token".into(),
                    label: "Bot token".into(),
                    kind: GatewayVarKind::String,
                    required: true,
                    secret: true,
                    default: None,
                    help: Some("Discord bot token".into()),
                }],
            }],
        };

        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["gateways"][0]["vars"][0]["key"], "bot_token");
        assert_eq!(json["gateways"][0]["vars"][0]["kind"], "String");
        assert_eq!(json["gateways"][0]["vars"][0]["required"], true);
        assert_eq!(json["gateways"][0]["vars"][0]["secret"], true);
    }
}
