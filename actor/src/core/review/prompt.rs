use crate::core::action::ActionKind;
use crate::core::prompt::context::{
    ConversationBacklogCtx, ReviewActionMessageCtx, ReviewDeliveryCtx, ReviewDueMemoryCtx,
    ReviewToolCallCtx, ReviewTranscriptCtx, ReviewTurnCtx,
};
use crate::core::social_read;
use crate::core::tools::SessionContext;
use crate::store::{
    ActionTranscriptRecord, ConversationSummary, Memory, PrivacyCategory, RecallQuery, Store,
    Thought,
};
use std::sync::Arc;

pub(crate) async fn fetch_review_due_memories(
    store: &Arc<dyn Store>,
    kind: &ActionKind,
    now: i64,
) -> Vec<ReviewDueMemoryCtx> {
    if !matches!(kind, ActionKind::Consolidate) {
        return vec![];
    }
    let query = RecallQuery::by_text("", 8)
        .with_next_review_due(now)
        .include_sensitive();
    let Ok(memories) = store.recall(&query).await else {
        return vec![];
    };
    memories
        .into_iter()
        .filter_map(|memory| review_due_memory_ctx(memory, now))
        .collect()
}

pub(crate) async fn fetch_conversation_backlog(
    store: &Arc<dyn Store>,
    kind: &ActionKind,
    now: i64,
) -> Vec<ConversationBacklogCtx> {
    if !matches!(kind, ActionKind::Consolidate) {
        return vec![];
    }

    let mut conversations = store.list_conversations().await.unwrap_or_default();
    conversations.retain(|conversation| conversation_summary_uncovered_count(conversation) > 0);
    conversations.sort_by(|a, b| {
        conversation_summary_uncovered_count(b)
            .cmp(&conversation_summary_uncovered_count(a))
            .then_with(|| b.last_message_at.cmp(&a.last_message_at))
    });
    conversations
        .into_iter()
        .take(8)
        .map(|conversation| conversation_backlog_ctx(conversation, now))
        .collect()
}

pub(crate) async fn fetch_review_transcript(
    store: &Arc<dyn Store>,
    kind: &ActionKind,
    session_ctx: &SessionContext,
) -> Option<ReviewTranscriptCtx> {
    if !matches!(kind, ActionKind::Review) {
        return None;
    }
    let source_action_id = review_source_action_id(session_ctx)?;
    let transcript = store.action_transcript(&source_action_id).await.ok()?;
    let thoughts = store
        .thoughts_for_action(&source_action_id, 20)
        .await
        .unwrap_or_default();
    Some(review_transcript_ctx(
        source_action_id,
        transcript,
        thoughts,
    ))
}

fn review_source_action_id(session_ctx: &SessionContext) -> Option<String> {
    session_ctx
        .cancelled_note
        .as_deref()
        .and_then(|note| note.strip_prefix("Post-turn review for action "))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn review_transcript_ctx(
    action_id: String,
    transcript: ActionTranscriptRecord,
    thoughts: Vec<Thought>,
) -> ReviewTranscriptCtx {
    let run = transcript.run;
    ReviewTranscriptCtx {
        action_id,
        kind: run.as_ref().map(|run| run.kind.clone()),
        task: run.as_ref().map(|run| run.task.clone()),
        status: run.as_ref().map(|run| run.status.clone()),
        responded: run.as_ref().map(|run| run.responded),
        attempts: run.as_ref().map(|run| run.attempts),
        messages: transcript
            .messages
            .into_iter()
            .map(|message| ReviewActionMessageCtx {
                role: message.role,
                speaker: message
                    .sender_external_id
                    .or(message.reply_external_id)
                    .or(message.source_gateway_id),
                source_message_id: message.source_message_id,
                content: message.content.map(|content| truncate_text(&content, 500)),
            })
            .collect(),
        turns: transcript
            .turns
            .into_iter()
            .map(|turn| ReviewTurnCtx {
                turn: turn.turn,
                attempt: turn.attempt,
                model: turn.model,
                finish: turn.finish,
                input_tokens: turn.input_tokens,
                output_tokens: turn.output_tokens,
                tool_call_count: turn.tool_call_count,
            })
            .collect(),
        tool_calls: transcript
            .tool_calls
            .into_iter()
            .map(|call| ReviewToolCallCtx {
                turn: call.turn,
                name: call.name,
                success: call.success,
                args: truncate_text(&call.args.to_string(), 700),
                result: truncate_text(&call.result.to_string(), 900),
            })
            .collect(),
        thoughts: thoughts.into_iter().map(social_read::thought_ctx).collect(),
        deliveries: transcript
            .deliveries
            .into_iter()
            .map(|delivery| ReviewDeliveryCtx {
                gateway_id: delivery.gateway_id,
                external_id: delivery.external_id,
                status: delivery.status,
                error: delivery.error.map(|error| truncate_text(&error, 300)),
            })
            .collect(),
        memories_formed: transcript
            .memories_formed
            .into_iter()
            .map(|id| id.0)
            .collect(),
        recalled_memory_ids: transcript
            .recalled_memory_ids
            .into_iter()
            .map(|id| id.0)
            .collect(),
    }
}

fn conversation_summary_uncovered_count(conversation: &ConversationSummary) -> u32 {
    conversation
        .message_count
        .saturating_sub(conversation.summary_covered_message_ids.len() as u32)
}

fn conversation_backlog_ctx(conversation: ConversationSummary, now: i64) -> ConversationBacklogCtx {
    let covered_message_count = conversation.summary_covered_message_ids.len() as u32;
    let uncovered_message_count = conversation_summary_uncovered_count(&conversation);
    ConversationBacklogCtx {
        ref_id: conversation.id.0,
        message_count: conversation.message_count,
        covered_message_count,
        uncovered_message_count,
        summary_version: conversation.summary_version,
        last_message: relative_duration(conversation.last_message_at, now),
        summary: conversation
            .summary
            .map(|summary| truncate_text(&summary, 220)),
    }
}

fn review_due_memory_ctx(memory: Memory, now: i64) -> Option<ReviewDueMemoryCtx> {
    let content = if review_due_memory_content_is_sensitive(&memory) {
        "[sensitive memory content redacted; use the memory id for retention, update, supersession, or deliberate authorized recall if needed]".into()
    } else {
        memory.content
    };
    Some(ReviewDueMemoryCtx {
        id: memory.id.0,
        memory_type: memory.memory_type.as_str().to_string(),
        truth_status: memory.truth_status.as_str().to_string(),
        due: describe_due_time(memory.next_review_at?, now),
        importance: pct(memory.importance),
        confidence: pct(memory.confidence),
        content,
    })
}

fn review_due_memory_content_is_sensitive(memory: &Memory) -> bool {
    matches!(
        memory.privacy_category,
        PrivacyCategory::Sensitive | PrivacyCategory::Secret
    )
}

fn relative_duration(from: i64, to: i64) -> String {
    let secs = (to - from).max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute ago".into()
        } else {
            format!("{m} minutes ago")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour ago".into()
        } else {
            format!("{h} hours ago")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day ago".into()
        } else {
            format!("{d} days ago")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week ago".into()
        } else {
            format!("{w} weeks ago")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month ago".into()
        } else {
            format!("{mo} months ago")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year ago".into()
        } else {
            format!("{y} years ago")
        }
    }
}

fn describe_due_time(fire_at: i64, now: i64) -> String {
    if fire_at <= now {
        let secs = (now - fire_at).max(0);
        if secs < 60 {
            "due now".into()
        } else {
            format!("overdue by {}", duration_words(secs))
        }
    } else {
        format!("in {}", duration_words(fire_at - now))
    }
}

fn duration_words(secs: i64) -> String {
    if secs < 60 {
        "less than 1 minute".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 {
            "1 minute".into()
        } else {
            format!("{m} minutes")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour".into()
        } else {
            format!("{h} hours")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day".into()
        } else {
            format!("{d} days")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week".into()
        } else {
            format!("{w} weeks")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month".into()
        } else {
            format!("{mo} months")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year".into()
        } else {
            format!("{y} years")
        }
    }
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
