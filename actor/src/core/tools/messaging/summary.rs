use super::target::current_conversation;
use super::*;

pub async fn update_conversation_summary(args: &Value, ctx: &SessionContext) -> String {
    let conv = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| current_conversation(ctx));
    let Some(conv) = conv else {
        return json!({
            "status": "error",
            "message": "No conversation specified and no current conversation.",
        })
        .to_string();
    };
    let Some(summary) = args["summary"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({
            "status": "error",
            "message": "Provide a non-empty summary.",
        })
        .to_string();
    };
    let covered_message_ids = if let Some(items) = args["covered_message_ids"].as_array() {
        items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>()
    } else {
        ctx.messages
            .iter()
            .map(|message| message.message_id.clone())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>()
    };

    match ctx
        .store
        .update_conversation_summary(&conv, summary, &covered_message_ids)
        .await
    {
        Ok(()) => json!({
            "status": "updated",
            "conversation": conv.0,
            "covered_message_ids": covered_message_ids,
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}
