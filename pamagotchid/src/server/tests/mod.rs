use super::*;
use actor::identity::{Group, GroupContext, Identity, Person, PersonProfileStatus, Profile};
use actor::store::{
    ActionMessageRecord, ActionRunRecord, ActionTurnRecord, Memory, MemoryKind, MemorySource,
    MemorySubject, OutboundDeliveryRecord, Store, ToolCallRecord,
};
use protocol::{GroupId, IdentityId, MemoryId, PersonId, ProfileId};

fn test_inbound(message_id: &str) -> InboundMessage {
    InboundMessage {
        message_id: message_id.into(),
        gateway_id: "relay".into(),
        sender_external_id: "local".into(),
        sender_display_name: None,
        reply_external_id: "local".into(),
        conversation: ConversationId("relay:local".into()),
        group: None,
        identity: None,
        profile: None,
        person: None,
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
