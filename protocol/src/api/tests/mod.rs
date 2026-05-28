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

#[test]
fn serializes_media_asset_created_event() {
    let event = ServerEvent::MediaAssetCreated {
        request_id: "req-media".into(),
        asset: MediaAssetView {
            id: MediaAssetId("media-1".into()),
            kind: MediaKind::Image,
            mime: Some("image/png".into()),
            filename: Some("image.png".into()),
            size: 3,
            sha256: "abc".into(),
        },
    };

    let json = serde_json::to_value(event).unwrap();

    assert_eq!(json["type"], "MediaAssetCreated");
    assert_eq!(json["request_id"], "req-media");
    assert_eq!(json["asset"]["id"], "media-1");
    assert_eq!(json["asset"]["kind"], "Image");
    assert_eq!(json["asset"]["mime"], "image/png");
}

#[test]
fn serializes_debug_snapshot_request_and_event() {
    let request = ClientRequest::GetDebugSnapshot {
        request_id: "req-debug".into(),
        limit: Some(5),
    };

    let json = serde_json::to_value(request).unwrap();

    assert_eq!(json["type"], "GetDebugSnapshot");
    assert_eq!(json["request_id"], "req-debug");
    assert_eq!(json["limit"], 5);

    let event = ServerEvent::DebugSnapshot {
        request_id: "req-debug".into(),
        snapshot: serde_json::json!({
            "persons": [],
            "action_runs": []
        }),
    };

    let json = serde_json::to_value(event).unwrap();

    assert_eq!(json["type"], "DebugSnapshot");
    assert_eq!(json["request_id"], "req-debug");
    assert_eq!(json["snapshot"]["persons"], serde_json::json!([]));
}
