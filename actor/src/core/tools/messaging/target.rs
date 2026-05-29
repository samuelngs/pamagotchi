use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeliveryTarget {
    pub gateway_id: String,
    pub external_id: String,
    pub channel: ChannelId,
}

pub(super) fn current_conversation(ctx: &SessionContext) -> Option<ConversationId> {
    ctx.conversation
        .clone()
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()))
}

pub(super) fn current_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages
        .first()
        .and_then(delivery_target_from_message)
        .map(|target| (target.gateway_id, target.external_id))
}

pub(super) async fn explicit_delivery_target(
    ctx: &SessionContext,
    gateway_id: &str,
    external_id: &str,
) -> Option<DeliveryTarget> {
    let gateway = GatewayId(gateway_id.to_string());
    ctx.store
        .resolve_channel(&gateway, external_id)
        .await
        .ok()
        .flatten()
        .filter(channel_allows_delivery)
        .map(delivery_target_from_channel)
}

pub(super) async fn default_delivery_target(ctx: &SessionContext) -> Option<DeliveryTarget> {
    if let Some(target) = ctx.messages.first().and_then(delivery_target_from_message) {
        return resolve_delivery_target(
            ctx,
            target.gateway_id.as_str(),
            target.external_id.as_str(),
        )
        .await;
    }

    let conversation = current_conversation(ctx)?;
    if let Some(channel) = ctx
        .store
        .channel_for_conversation(&conversation)
        .await
        .ok()?
    {
        if !channel_allows_delivery(&channel) {
            return None;
        }
        return Some(DeliveryTarget {
            gateway_id: channel.gateway.0,
            external_id: channel.external_id,
            channel: channel.id,
        });
    }
    None
}

pub(super) async fn outbound_relationship_person(
    ctx: &SessionContext,
    conversation: Option<&ConversationId>,
) -> Option<PersonId> {
    if let Some(person) = ctx.messages.first().and_then(|msg| msg.person.clone()) {
        return Some(person);
    }

    let conversation = conversation?;
    ctx.store
        .list_conversations()
        .await
        .ok()?
        .into_iter()
        .find(|summary| summary.id == *conversation)
        .and_then(|summary| summary.person)
}

async fn resolve_delivery_target(
    ctx: &SessionContext,
    gateway_id: &str,
    external_id: &str,
) -> Option<DeliveryTarget> {
    let gateway = GatewayId(gateway_id.to_string());
    ctx.store
        .resolve_channel(&gateway, external_id)
        .await
        .ok()
        .flatten()
        .filter(channel_allows_delivery)
        .map(delivery_target_from_channel)
}

fn channel_allows_delivery(channel: &ChannelRecord) -> bool {
    !channel
        .metadata
        .get("delivery_supported")
        .and_then(|value| value.as_bool())
        .is_some_and(|supported| !supported)
}

fn delivery_target_from_channel(channel: ChannelRecord) -> DeliveryTarget {
    DeliveryTarget {
        gateway_id: channel.gateway.0,
        external_id: channel.external_id,
        channel: channel.id,
    }
}

fn delivery_target_from_message(msg: &InboundMessage) -> Option<DeliveryTarget> {
    if let Some(envelope) = msg
        .metadata
        .get("normalized_envelope")
        .and_then(|value| serde_json::from_value::<InboundEnvelope>(value.clone()).ok())
    {
        return Some(DeliveryTarget {
            gateway_id: envelope.gateway_id.0.clone(),
            external_id: envelope.channel.external_id.clone(),
            channel: protocol::channel_id(
                &envelope.channel.gateway_id,
                envelope.channel.external_id.as_str(),
            ),
        });
    }

    msg.reply_target().map(|(gateway, target)| DeliveryTarget {
        gateway_id: gateway.to_string(),
        external_id: target.to_string(),
        channel: msg.channel_id(),
    })
}
