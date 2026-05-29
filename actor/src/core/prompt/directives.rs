use super::*;

pub(super) async fn load_directives(
    store: &Arc<dyn Store>,
    actor: &ActorState,
    person: &protocol::PersonId,
    conversation: Option<&ConversationId>,
    current_msg: Option<&InboundMessage>,
) -> anyhow::Result<Vec<String>> {
    let rel = actor.bonds.get(person);
    let relationship_standing = rel.map_or(RelationshipStanding::Default, |r| {
        r.relationship_standing.clone()
    });

    let channel = if let Some(message) = current_msg {
        Some(message.channel_id())
    } else if let Some(conv) = conversation {
        store
            .channel_for_conversation(conv)
            .await?
            .map(|channel| channel.id)
    } else {
        None
    };

    let directives = store
        .get_directives_for_context(person, &relationship_standing, channel.as_ref())
        .await?;
    Ok(directives.into_iter().map(|d| d.directive).collect())
}
