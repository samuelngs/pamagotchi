use super::identity::{resolve_gateway_person, resolve_relay_person};
use super::observation::observe_group_membership;
use crate::core::handle::StateHandle;
use crate::store::Store;
use protocol::InboundMessage;
use std::sync::Arc;

pub(crate) async fn resolve_person(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    msg: &mut InboundMessage,
) {
    if msg.identity.is_some() && msg.profile.is_some() {
        observe_group_membership(store, msg).await;
        return;
    }
    if msg.gateway_id == "relay" {
        resolve_relay_person(state, store, msg).await;
    } else {
        resolve_gateway_person(state, store, msg).await;
    }
    observe_group_membership(store, msg).await;
}
