use super::*;

pub(super) async fn load_directives(
    store: &Arc<dyn Store>,
    actor: &ActorState,
    person: &protocol::PersonId,
    conversation: Option<&ConversationId>,
) -> anyhow::Result<Vec<String>> {
    let rel = actor.bonds.get(person);
    let authority = rel.map_or(Authority::Default, |r| r.authority.clone());

    let group = if let Some(conv) = conversation {
        let summaries = store.list_conversations().await?;
        summaries
            .into_iter()
            .find(|s| s.id == *conv)
            .and_then(|s| s.group)
    } else {
        None
    };

    let directives = store
        .get_directives_for_context(person, &authority, group.as_ref())
        .await?;
    Ok(directives.into_iter().map(|d| d.directive).collect())
}
