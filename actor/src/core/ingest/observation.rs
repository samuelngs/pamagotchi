use crate::core::handle::StateHandle;
use crate::core::tools::empty_delta;
use crate::state::{RelationshipChange, RelationshipInteraction};
use crate::store::{ChannelMembership, ChannelMembershipStatus, Store};
use protocol::InboundMessage;
use std::sync::Arc;
use tracing::warn;

pub(crate) async fn observe_inbound(state: &StateHandle, msg: &InboundMessage) {
    let Some(person) = msg.person.clone() else {
        return;
    };
    let mut delta = empty_delta(Some(person.clone()));
    delta.relationship_changes.push(RelationshipChange {
        person,
        trust_delta: 0.0,
        trust_ceiling: None,
        familiarity_delta: 0.002,
        valence_delta: 0.0,
        proactive_consent: None,
        response_cadence: None,
        channel_preference: None,
        interaction: Some(RelationshipInteraction::Inbound),
    });
    state.send_delta(delta).await;
}

pub(super) async fn observe_channel_membership(store: &Arc<dyn Store>, msg: &InboundMessage) {
    let Some(profile) = msg.profile.as_ref() else {
        return;
    };
    let channel = msg.channel_id();
    let platform_message_id = msg
        .metadata
        .get("normalized_envelope")
        .and_then(|value| {
            value
                .get("platform_message_id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| msg.message_id.clone());
    if let Err(e) = store
        .upsert_channel_membership(&ChannelMembership {
            channel: channel.clone(),
            profile: profile.clone(),
            role: None,
            status: ChannelMembershipStatus::Observed,
            first_seen_at: msg.timestamp,
            last_seen_at: msg.timestamp,
            metadata: serde_json::json!({
                "source": "inbound_message",
                "platform_message_id": platform_message_id,
                "sender": msg.sender,
            }),
        })
        .await
    {
        warn!(
            %e,
            channel = %channel.0,
            profile = %profile.0,
            "failed to record observed channel membership"
        );
    }
}
