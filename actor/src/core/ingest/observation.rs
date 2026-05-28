use crate::core::handle::StateHandle;
use crate::core::tools::empty_delta;
use crate::identity::{Group, GroupContext};
use crate::state::{RelationshipChange, RelationshipInteraction};
use crate::store::Store;
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

pub(super) async fn observe_group_membership(store: &Arc<dyn Store>, msg: &InboundMessage) {
    let (Some(group), Some(person)) = (msg.group.as_ref(), msg.person.as_ref()) else {
        return;
    };

    match store.get_group(group).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            let group_record = Group {
                id: group.clone(),
                name: msg
                    .metadata
                    .get("group_name")
                    .or_else(|| msg.metadata.get("guild_name"))
                    .and_then(|value| value.as_str())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(&group.0)
                    .to_string(),
                gateway_id: msg.gateway_id.clone(),
                external_id: msg
                    .metadata
                    .get("group_id")
                    .or_else(|| msg.metadata.get("guild_id"))
                    .and_then(|value| value.as_str())
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or(&group.0)
                    .to_string(),
                context: GroupContext::Social,
                members: vec![],
            };
            if let Err(e) = store.add_group(&group_record).await {
                warn!(%e, group = %group.0, "failed to create observed group");
            }
        }
        Err(e) => warn!(%e, group = %group.0, "failed to load observed group"),
    }

    if let Err(e) = store.add_group_member(group, person).await {
        warn!(
            %e,
            group = %group.0,
            person = %person.0,
            "failed to record observed group member"
        );
    }
}
