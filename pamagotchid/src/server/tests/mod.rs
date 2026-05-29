use super::*;
use actor::identity::{Group, GroupContext, Identity, Person, PersonProfileStatus, Profile};
use actor::store::{
    ActionMessageRecord, ActionRunRecord, ActionTurnRecord, Memory, MemoryKind, MemorySource,
    MemorySubject, OutboundDeliveryRecord, Store, ToolCallRecord,
};
use protocol::{GroupId, IdentityId, MemoryId, PersonId, ProfileId};

fn test_inbound(message_id: &str) -> InboundEnvelope {
    let gateway_id = protocol::GatewayId("relay".into());
    InboundEnvelope {
        gateway_id: gateway_id.clone(),
        platform_message_id: message_id.into(),
        channel: protocol::ChannelKey {
            gateway_id: gateway_id.clone(),
            external_id: "local".into(),
            kind: protocol::ChannelKind::RelayRoom,
            display_name: None,
            space: None,
            parent: None,
            metadata: serde_json::Value::Null,
        },
        sender: Some(protocol::ObservedSender {
            primary: protocol::ObservedIdentityKey {
                gateway_id,
                external_id: "local".into(),
                kind: Some("relay_user".into()),
                confidence: 1.0,
                source: "primary_sender".into(),
            },
            aliases: vec![],
            display_name: None,
            metadata: serde_json::Value::Null,
        }),
        content: "hello".into(),
        attachments: Vec::new(),
        timestamp: now_secs(),
        metadata: serde_json::Value::Null,
    }
}

mod debug_snapshot_tests;
mod encoding_tests;
mod gateway_event_tests;
mod inbound_bridge_tests;
