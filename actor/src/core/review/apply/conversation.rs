use super::*;

pub(super) async fn apply_conversation_summary(
    args: &Value,
    ctx: &SessionContext,
    counts: &mut ApplyCounts,
) {
    let summary_obj = &args["conversation_summary"];
    if !summary_obj.is_object() {
        return;
    }
    let Some(summary) = summary_obj["summary"]
        .as_str()
        .filter(|summary| !summary.trim().is_empty())
    else {
        counts
            .skipped
            .push("conversation_summary missing summary".into());
        return;
    };
    let Some(conversation) = summary_obj["conversation_id"]
        .as_str()
        .map(|id| ConversationId(id.to_string()))
        .or_else(|| ctx.conversation.clone())
    else {
        counts
            .skipped
            .push("conversation_summary missing conversation".into());
        return;
    };
    let covered = string_array(&summary_obj["covered_message_ids"]).collect::<Vec<_>>();
    let existing = ctx
        .store
        .list_conversations()
        .await
        .ok()
        .and_then(|conversations| {
            conversations
                .into_iter()
                .find(|candidate| candidate.id == conversation)
        });
    let summary_update = merge_summary_update(
        existing
            .as_ref()
            .and_then(|conversation| conversation.summary.as_deref()),
        summary,
    );
    let existing_covered = existing
        .as_ref()
        .map(|conversation| conversation.summary_covered_message_ids.clone())
        .unwrap_or_default();
    let merged_covered = merge_ordered_ids(existing_covered.clone(), covered);
    let covered_changed = merged_covered != existing_covered;
    let summary_to_store = summary_update.or_else(|| {
        covered_changed.then(|| {
            existing
                .as_ref()
                .and_then(|conversation| conversation.summary.clone())
                .unwrap_or_else(|| summary.to_string())
        })
    });
    let Some(summary_to_store) = summary_to_store else {
        counts.skipped.push(format!(
            "conversation_summary {} had no new fields",
            conversation.0
        ));
        return;
    };
    match ctx
        .store
        .update_conversation_summary(&conversation, &summary_to_store, &merged_covered)
        .await
    {
        Ok(()) => counts.conversation_summaries += 1,
        Err(e) => counts
            .skipped
            .push(format!("conversation_summary failed: {e}")),
    }
}
