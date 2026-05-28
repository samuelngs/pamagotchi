use super::*;

pub(super) fn current_conversation(ctx: &SessionContext) -> Option<ConversationId> {
    ctx.conversation
        .clone()
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()))
}

pub(super) fn current_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages
        .first()
        .and_then(|msg| msg.reply_target())
        .map(|(gateway, target)| (gateway.to_string(), target.to_string()))
}

pub(super) async fn default_delivery_target(ctx: &SessionContext) -> Option<(String, String)> {
    if let Some((gateway, target)) = ctx.messages.first().and_then(|msg| msg.reply_target()) {
        return Some((gateway.to_string(), target.to_string()));
    }

    let conversation = current_conversation(ctx)?;
    let conversation_gateway =
        ctx.store
            .list_conversations()
            .await
            .ok()
            .and_then(|conversations| {
                conversations
                    .into_iter()
                    .find(|summary| summary.id == conversation)
                    .and_then(|summary| summary.gateway_id)
            });
    let messages = ctx.store.get_messages(&conversation, 20, None).await.ok()?;

    messages.iter().rev().find_map(|message| {
        let target = message.reply_external_id.as_ref()?;
        let gateway = message
            .source_gateway_id
            .as_ref()
            .or(conversation_gateway.as_ref())?;
        Some((gateway.clone(), target.clone()))
    })
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
