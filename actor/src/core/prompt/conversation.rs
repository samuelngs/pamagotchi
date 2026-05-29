use super::person::resolve_person_info;
use super::*;

pub(super) async fn fetch_conversation_summary(
    store: &Arc<dyn Store>,
    conversation: Option<&ConversationId>,
) -> Option<ConversationSummary> {
    let conversation = conversation?;
    store
        .list_conversations()
        .await
        .ok()?
        .into_iter()
        .find(|summary| summary.id == *conversation)
}

pub(super) fn conversation_ctx_from_summary(summary: &ConversationSummary) -> ConversationCtx {
    ConversationCtx {
        ref_id: summary.id.0.clone(),
        summary: summary.summary.clone(),
    }
}

pub(super) async fn fetch_current_channel_ctx(
    store: &Arc<dyn Store>,
    current_msg: Option<&InboundMessage>,
    conversation: Option<&ConversationId>,
) -> Option<ChannelCtx> {
    if let Some(message) = current_msg {
        return Some(ChannelCtx {
            ref_id: message.channel_id().0,
            gateway_id: message.channel.gateway_id.0.clone(),
            external_id: message.channel.external_id.clone(),
            kind: message.channel.kind.as_str().to_string(),
            display_name: message.channel.display_name.clone(),
        });
    }

    let conversation = conversation?;
    store
        .channel_for_conversation(conversation)
        .await
        .ok()
        .flatten()
        .map(|channel| ChannelCtx {
            ref_id: channel.id.0,
            gateway_id: channel.gateway.0,
            external_id: channel.external_id,
            kind: channel.kind.as_str().to_string(),
            display_name: channel.display_name,
        })
}

pub(super) async fn fetch_group_ctx(
    store: &Arc<dyn Store>,
    group_id: Option<&GroupId>,
) -> Option<GroupCtx> {
    let group_id = group_id?;
    match store.get_group(group_id).await {
        Ok(Some(group)) => {
            let mut members = Vec::new();
            for member in group.members.iter().take(12) {
                let info = resolve_person_info(store, member).await;
                members.push(GroupMemberCtx {
                    ref_id: member.0.clone(),
                    name: info.name,
                });
            }
            members.sort_by(|a, b| {
                a.name
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.name.as_deref().unwrap_or(""))
                    .then_with(|| a.ref_id.cmp(&b.ref_id))
            });
            Some(GroupCtx {
                ref_id: group.id.0,
                name: Some(group.name),
                gateway_id: Some(group.gateway_id),
                external_id: Some(group.external_id),
                context: Some(group.context.as_str().to_string()),
                member_count: group.members.len(),
                members,
            })
        }
        Ok(None) => Some(GroupCtx {
            ref_id: group_id.0.clone(),
            name: None,
            gateway_id: None,
            external_id: None,
            context: None,
            member_count: 0,
            members: vec![],
        }),
        Err(_) => None,
    }
}
