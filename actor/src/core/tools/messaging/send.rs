use super::attachments::{outbound_metadata, parse_attachments};
use super::delivery::notify_chosen_human_of_delivery_failure;
use super::target::{
    current_composing_target, current_conversation, default_delivery_target,
    explicit_delivery_target, outbound_relationship_person,
};
use super::typing::wait_if_current_sender_is_typing;
use super::*;
use crate::core::tools::util;

pub async fn send(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let content = normalize_outbound_text(args["content"].as_str().unwrap_or(""));
    let gateway_id = args["gateway_id"].as_str();
    let external_id = args["external_id"].as_str();

    let attachments = match parse_attachments(args) {
        Ok(attachments) => attachments,
        Err(message) => return message,
    };

    let is_outbound = gateway_id.is_some() && external_id.is_some();

    let target = if is_outbound {
        match explicit_delivery_target(ctx, gateway_id.unwrap(), external_id.unwrap()).await {
            Some(target) => target,
            None => return "No delivery target — message not sent.".into(),
        }
    } else if let Some(target) = default_delivery_target(ctx).await {
        target
    } else {
        return "No delivery target — message not sent.".into();
    };
    let target_gateway = target.gateway_id.clone();
    let target_id = target.external_id.clone();

    let conversation = current_conversation(ctx);
    let outbound_message_id = generated_message_id();
    if !is_outbound {
        wait_if_current_sender_is_typing(ctx).await;
    }
    state.attempted_send = true;
    let delivery = ctx
        .gateway
        .send_message(&target_gateway, &target_id, &content, &attachments)
        .await;

    if !state.composing_released
        && current_composing_target(ctx)
            .as_ref()
            .is_some_and(|(gateway, id)| gateway == &target_gateway && id == &target_id)
    {
        ctx.gateway
            .release_composing(&target_gateway, &target_id)
            .await;
        state.composing_released = true;
    }

    let attempted_at = util::now();
    let action_message = ActionMessageRecord {
        action_id: ctx.action_id.0.clone(),
        role: if delivery.is_ok() {
            "assistant".into()
        } else {
            "assistant_delivery_failed".into()
        },
        conversation: conversation.clone(),
        source_gateway_id: None,
        source_message_id: None,
        sender_external_id: None,
        reply_external_id: Some(target_id.clone()),
        content: Some(content.clone()),
        created_at: attempted_at,
    };
    if let Err(e) = ctx.store.append_action_message(&action_message).await {
        warn!(
            action = %ctx.action_id,
            %e,
            "failed to persist assistant action message link"
        );
    }

    if delivery.is_ok() {
        if let Some(conv) = conversation.clone() {
            let stored = StoredMessage {
                timestamp: attempted_at,
                role: MessageRole::Assistant,
                content: content.clone(),
                identity: None,
                profile: None,
                person: None,
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some(target_id.clone()),
                metadata: {
                    let mut metadata = match outbound_metadata(&attachments) {
                        serde_json::Value::Object(obj) => serde_json::Value::Object(obj),
                        _ => serde_json::json!({}),
                    };
                    if let serde_json::Value::Object(obj) = &mut metadata {
                        obj.insert("message_id".into(), json!(outbound_message_id.0.clone()));
                        obj.insert("channel_id".into(), json!(target.channel.0.clone()));
                    }
                    metadata
                },
            };
            ctx.store.append_message(&conv, &stored).await.ok();
        }
    }

    let (delivery_status, delivery_error) = match &delivery {
        Ok(_) => ("delivered", None),
        Err(e) => ("failed", Some(e.to_string())),
    };
    ctx.metrics.record_outbound_delivery(delivery.is_ok());
    if let Err(e) = ctx
        .store
        .append_outbound_delivery(&OutboundDeliveryRecord {
            action_id: ctx.action_id.0.clone(),
            conversation: conversation.clone(),
            message: Some(outbound_message_id),
            channel: Some(target.channel.clone()),
            gateway_id: target_gateway.clone(),
            external_id: target_id.clone(),
            status: delivery_status.into(),
            error: delivery_error,
            attempted_at,
        })
        .await
    {
        warn!(
            action = %ctx.action_id,
            %e,
            "failed to persist outbound delivery status"
        );
    }

    match delivery {
        Ok(_) => {
            state.responded = true;
            if let Some(person) = outbound_relationship_person(ctx, conversation.as_ref()).await {
                let interaction = if matches!(ctx.kind, SessionKind::Action(ActionKind::Outreach)) {
                    RelationshipInteraction::ProactiveOutbound
                } else {
                    RelationshipInteraction::Outbound
                };
                state.delta.relationship_changes.push(RelationshipChange {
                    person,
                    trust_delta: 0.0,
                    trust_ceiling: None,
                    familiarity_delta: 0.002,
                    valence_delta: 0.0,
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: Some(interaction),
                });
            }
            if is_outbound {
                format!("Message sent to {target_gateway}:{target_id}.")
            } else {
                "Message sent.".into()
            }
        }
        Err(e) => {
            let error = e.to_string();
            let supervisor_notified = notify_chosen_human_of_delivery_failure(
                ctx,
                &target_gateway,
                &target_id,
                conversation.as_ref(),
                &content,
                &error,
                attempted_at,
            )
            .await;
            warn!(
                action = %ctx.action_id,
                error = %error,
                gateway = %target_gateway,
                "message delivery failed"
            );
            if supervisor_notified {
                format!(
                    "Delivery failed; message was not added to visible conversation history and is not marked delivered. Chosen-human review is queued: {error}"
                )
            } else {
                format!(
                    "Delivery failed; message was not added to visible conversation history and is not marked delivered: {error}"
                )
            }
        }
    }
}

fn normalize_outbound_text(content: &str) -> String {
    let mut normalized = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '\u{2014}' | '\u{2013}') {
            while normalized.ends_with(' ') {
                normalized.pop();
            }
            if !normalized.is_empty() {
                normalized.push(',');
            }
            if chars.peek().is_some_and(|next| !next.is_whitespace()) {
                normalized.push(' ');
            }
        } else {
            normalized.push(ch);
        }
    }
    normalized
}
