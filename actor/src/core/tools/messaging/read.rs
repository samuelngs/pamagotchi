use super::target::current_conversation;
use super::*;

#[cfg(test)]
pub async fn read(args: &Value, ctx: &SessionContext) -> String {
    read_inner(args, ctx, None).await
}

pub async fn read_with_state(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> String {
    read_inner(args, ctx, Some(state)).await
}

async fn read_inner(
    args: &Value,
    ctx: &SessionContext,
    mut state: Option<&mut SessionState>,
) -> String {
    let conv = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| current_conversation(ctx));

    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let before = args["before"].as_i64();
    let Some(conv) = conv else {
        if can_read_recent_without_current_conversation(ctx) {
            return read_recent_conversations(ctx, state.as_deref_mut(), limit, before).await;
        }
        return "No conversation specified and no current conversation.".into();
    };

    read_conversation_messages(ctx, state.as_deref_mut(), &conv, limit, before).await
}

async fn read_conversation_messages(
    ctx: &SessionContext,
    state: Option<&mut SessionState>,
    conv: &ConversationId,
    limit: usize,
    before: Option<i64>,
) -> String {
    match ctx.store.get_messages(conv, limit, before).await {
        Ok(messages) if messages.is_empty() => json!({"messages": []}).to_string(),
        Ok(messages) => {
            let (gateway_hint, group) = conversation_evidence_context(ctx, conv).await;
            record_read_evidence(
                state,
                &ctx.messages,
                conv,
                gateway_hint.as_deref(),
                group.as_ref(),
                &messages,
            );
            let mut items = Vec::new();
            for message in &messages {
                items.push(message_json(ctx, message).await);
            }
            json!({"messages": items}).to_string()
        }
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}

async fn read_recent_conversations(
    ctx: &SessionContext,
    mut state: Option<&mut SessionState>,
    limit: usize,
    before: Option<i64>,
) -> String {
    let mut remaining = limit.max(1).min(20);
    let conversations = match ctx.store.list_conversations().await {
        Ok(conversations) => conversations,
        Err(e) => return json!({"error": format!("{e}")}).to_string(),
    };

    let mut out = Vec::new();
    for conversation in conversations.into_iter().take(8) {
        if remaining == 0 {
            break;
        }
        let read_limit = remaining.min(5);
        let Ok(messages) = ctx
            .store
            .get_messages(&conversation.id, read_limit, before)
            .await
        else {
            continue;
        };
        if messages.is_empty() {
            continue;
        }
        record_read_evidence(
            state.as_deref_mut(),
            &ctx.messages,
            &conversation.id,
            conversation.gateway_id.as_deref(),
            conversation.group.as_ref(),
            &messages,
        );
        remaining = remaining.saturating_sub(messages.len());
        let mut items = Vec::new();
        for message in &messages {
            items.push(message_json(ctx, message).await);
        }
        out.push(json!({
            "conversation": conversation.id.0,
            "gateway_id": conversation.gateway_id,
            "group": conversation.group.map(|group| group.0),
            "summary": conversation.summary,
            "messages": items,
        }));
    }

    json!({"conversations": out}).to_string()
}

async fn conversation_evidence_context(
    ctx: &SessionContext,
    conv: &ConversationId,
) -> (Option<String>, Option<GroupId>) {
    ctx.store
        .list_conversations()
        .await
        .ok()
        .and_then(|conversations| {
            conversations
                .into_iter()
                .find(|summary| summary.id == *conv)
        })
        .map(|summary| (summary.gateway_id, summary.group))
        .unwrap_or((None, None))
}

fn record_read_evidence(
    state: Option<&mut SessionState>,
    current_messages: &[InboundMessage],
    conversation: &ConversationId,
    gateway_hint: Option<&str>,
    group: Option<&GroupId>,
    messages: &[StoredMessage],
) {
    let Some(state) = state else {
        return;
    };

    for message in messages {
        let evidence = read_evidence_message(conversation, gateway_hint, group, message);
        let id = evidence.message_id.as_str();
        let already_presented = state
            .presented_read_messages
            .iter()
            .chain(state.presented_injected_messages.iter())
            .chain(current_messages.iter())
            .any(|message| message.message_id == id);
        if !already_presented {
            state.presented_read_messages.push(evidence);
        }
    }
}

fn read_evidence_message(
    conversation: &ConversationId,
    gateway_hint: Option<&str>,
    group: Option<&GroupId>,
    message: &StoredMessage,
) -> InboundMessage {
    InboundMessage {
        message_id: message.readable_message_id(),
        gateway_id: message
            .source_gateway_id
            .clone()
            .or_else(|| gateway_hint.map(str::to_string))
            .unwrap_or_default(),
        sender_external_id: message.sender_external_id.clone().unwrap_or_default(),
        sender_display_name: None,
        reply_external_id: message.reply_external_id.clone().unwrap_or_default(),
        conversation: conversation.clone(),
        group: group.cloned(),
        identity: message.identity.clone(),
        profile: message.profile.clone(),
        person: message.person.clone(),
        content: message.content.clone(),
        attachments: vec![],
        timestamp: message.timestamp,
        metadata: message.metadata.clone(),
    }
}

fn can_read_recent_without_current_conversation(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(
                ActionKind::Review | ActionKind::Consolidate | ActionKind::Ruminate
            )
        )
}

async fn message_json(ctx: &SessionContext, m: &StoredMessage) -> Value {
    let ts = chrono::DateTime::from_timestamp(m.timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| m.timestamp.to_string());
    let from = if matches!(m.role, MessageRole::Assistant) {
        json!({"role": "self"})
    } else {
        let mut f = json!({"role": "user"});
        if let Some(pid) = &m.person {
            f["ref"] = json!(pid.0);
            if let Ok(Some(p)) = ctx.store.get_person(pid).await {
                if let Some(name) = &p.name {
                    f["name"] = json!(name);
                }
            }
        }
        f
    };
    let mut item = json!({
        "message_id": m.readable_message_id(),
        "time": ts,
        "from": from,
        "content": m.content,
    });
    if m.source_gateway_id.is_some() || m.source_message_id.is_some() {
        item["source"] = json!({
            "gateway_id": m.source_gateway_id.as_deref(),
            "message_id": m.source_message_id.as_deref(),
        });
    }
    if let Some(attachments) = m.metadata.get("attachments") {
        item["attachments"] = attachments.clone();
    }
    item
}
