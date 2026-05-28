mod messages;
mod stream;
mod tool_calls;

#[cfg(test)]
mod tests;

use super::action::Outcome;
use super::decision::MindVerdict;
use super::tools::{self, SessionContext, SessionKind, SessionState};
use crate::store::{ActionPromptSnapshotRecord, ActionTurnRecord};
use gateway::GatewayRouter;
use inference::{AssistantMessage, ContentPart, FinishReason, Message, RouteContext, UserMessage};
use messages::{
    build_prompt, ingest_messages, inject_pending_messages, remember_injected_message,
    required_capabilities, resolve_composing_target, source_message_keys, user_message_for_inbound,
};
use protocol::{ConversationId, PersonId};
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use stream::{collect_stream, log_turn_start, try_open_stream};
use tool_calls::{execute_tools, finalize_tool_calls};
use tracing::{info, warn};

pub struct OutboundMessage {
    pub conversation: ConversationId,
    pub content: String,
    pub person: Option<PersonId>,
}

pub enum SessionResult {
    Mind(MindVerdict),
    Action(Outcome),
}

pub async fn run_session(mut ctx: SessionContext) -> SessionResult {
    let cancellation = session_cancellation_token(&ctx);
    let mut composing_guard =
        if let Some((gateway_id, external_id)) = resolve_composing_target(&ctx) {
            ctx.gateway
                .acquire_composing(&gateway_id, &external_id)
                .await;
            Some(ComposingGuard::new(
                ctx.gateway.clone(),
                gateway_id,
                external_id,
            ))
        } else {
            None
        };

    let expects_response = matches!(&ctx.kind, SessionKind::Action(k) if k.expects_response());
    let max_attempts = if expects_response {
        ctx.max_action_attempts
    } else {
        1
    };
    let escalate_after = ctx.escalate_after;

    let mut attempt = 0;
    let mut cancelled = false;
    let mut mind_verdict: Option<MindVerdict> = None;
    let mut state = SessionState {
        responded: false,
        attempted_send: false,
        composing_released: false,
        delta: tools::empty_delta(ctx.messages.first().and_then(|m| m.person.clone())),
        thoughts: vec![],
        memories_formed: vec![],
        recalled_memory_ids: vec![],
        injected_messages: vec![],
        presented_injected_messages: vec![],
        presented_read_messages: vec![],
        pending_injected_messages: vec![],
        source_message_keys: source_message_keys(&ctx.messages),
        queued_injected_message_keys: Default::default(),
        presented_injected_message_keys: Default::default(),
        applied_review_keys: Default::default(),
        presented_injection_count: 0,
    };

    'session: loop {
        if cancellation.is_cancelled() {
            cancelled = true;
            break;
        }
        attempt += 1;

        if attempt > 1 && expects_response {
            let new_reasoning = if attempt > escalate_after {
                let escalated = ctx.reasoning.escalate();
                if escalated != ctx.reasoning {
                    info!(
                        action = %ctx.action_id, attempt,
                        from = ?ctx.reasoning, to = ?escalated,
                        "escalating reasoning tier"
                    );
                    ctx.reasoning = escalated;
                    ctx.endpoints = ctx.router.resolve_chain(&RouteContext::Action(escalated));
                }
                escalated
            } else {
                ctx.reasoning
            };
            info!(
                action = %ctx.action_id, attempt,
                reasoning = ?new_reasoning,
                "retrying action with warning"
            );
        }

        let retry_warning = if attempt > 1 {
            Some(
                "IMPORTANT: Your previous attempt failed to call send_message. You MUST use send_message to respond. Text outside of tool calls is silent inner thought that no one can see or hear.",
            )
        } else {
            None
        };

        let system_prompt = match build_prompt(&ctx).await {
            Ok(p) => p,
            Err(e) => {
                warn!(%e, action = %ctx.action_id, "failed to build prompt");
                break;
            }
        };

        info!(action = %ctx.action_id, system_prompt_len = system_prompt.len(), "system prompt built");
        tracing::debug!(
            action = %ctx.action_id,
            system_prompt_len = system_prompt.len(),
            "system prompt content redacted from logs"
        );

        let mut llm_messages = vec![Message::system(system_prompt)];
        if let Some(warning) = retry_warning {
            llm_messages.push(Message::system(warning));
        }
        let required_caps = required_capabilities(&ctx.messages, &state.injected_messages);
        if !required_caps.is_empty() && !ctx.router.chat_supports(&required_caps) {
            llm_messages.push(Message::system(
                "The current inference configuration cannot inspect visual attachments. Use the visible attachment metadata only; do not claim to have seen image, video, or sticker contents.",
            ));
        }
        if attempt == 1 {
            ingest_messages(&ctx, &mut llm_messages).await;
        } else {
            for inbound in &ctx.messages {
                llm_messages.push(user_message_for_inbound(&ctx, inbound).await);
            }
        }

        let tool_defs = match &ctx.kind {
            SessionKind::Mind => tools::mind_tools(),
            SessionKind::Action(kind) => tools::action_tools(kind),
        };

        state.responded = false;

        for turn in 0..ctx.max_turns {
            if cancellation.is_cancelled() {
                cancelled = true;
                break 'session;
            }
            log_turn_start(&ctx, turn, &llm_messages);
            let prompt_hash = prompt_hash(&llm_messages);
            let snapshot_time = tools::util::now();
            let prompt_snapshot = ActionPromptSnapshotRecord {
                action_id: ctx.action_id.0.clone(),
                turn: turn as u32,
                attempt: attempt as u32,
                prompt_hash: prompt_hash.clone(),
                messages: prompt_snapshot_messages(&llm_messages),
                created_at: snapshot_time,
            };
            if let Err(e) = ctx.store.record_prompt_snapshot(&prompt_snapshot).await {
                warn!(
                    %e,
                    action = %ctx.action_id,
                    turn,
                    "failed to persist prompt snapshot"
                );
            }

            let required_caps = required_capabilities(&ctx.messages, &state.injected_messages);
            let mut opened = tokio::select! {
                opened = try_open_stream(&ctx, &llm_messages, &tool_defs, &required_caps) => {
                    match opened {
                        Some(s) => s,
                        None => break,
                    }
                }
                _ = cancellation.cancelled() => {
                    cancelled = true;
                    break 'session;
                }
            };
            let model = opened.model.clone();

            let collected = tokio::select! {
                collected = collect_stream(&mut opened, &mut ctx, &mut state, turn) => collected,
                _ = cancellation.cancelled() => {
                    cancelled = true;
                    break 'session;
                }
            };
            info!(action = %ctx.action_id, turn, "LLM stream ended");

            let tool_calls = finalize_tool_calls(collected.partial_tools);
            let turn_record = ActionTurnRecord {
                action_id: ctx.action_id.0.clone(),
                turn: turn as u32,
                attempt: attempt as u32,
                prompt_hash,
                model: Some(model.clone()),
                finish: Some(finish_reason_name(&collected.finish).into()),
                input_tokens: collected.input_tokens,
                output_tokens: collected.output_tokens,
                text_len: collected.text.len() as u32,
                reasoning_len: collected.reasoning.len() as u32,
                tool_call_count: tool_calls.len() as u32,
                created_at: tools::util::now(),
            };
            if let Err(e) = ctx.store.append_action_turn(&turn_record).await {
                warn!(%e, action = %ctx.action_id, turn, "failed to persist action turn");
            }
            if let (Some(input_tokens), Some(output_tokens)) =
                (collected.input_tokens, collected.output_tokens)
            {
                ctx.metrics
                    .record_prompt_tokens(input_tokens, output_tokens);
            }

            info!(
                action = %ctx.action_id, turn,
                finish = ?collected.finish, tool_calls = tool_calls.len(),
                text_len = collected.text.len(), "LLM turn complete"
            );

            if !collected.text.is_empty() {
                info!(action = %ctx.action_id, thought = %collected.text, "internal monologue");
            }

            if let Some(verdict) = collected.app_server_decision {
                mind_verdict = Some(verdict);
                break;
            }

            if tool_calls.is_empty() {
                break;
            }

            llm_messages.push(Message::Assistant(AssistantMessage {
                text: if collected.text.is_empty() {
                    None
                } else {
                    Some(collected.text)
                },
                reasoning_content: if collected.reasoning.is_empty() {
                    None
                } else {
                    Some(collected.reasoning)
                },
                tool_calls: tool_calls.clone(),
            }));

            if cancellation.is_cancelled() {
                cancelled = true;
                break 'session;
            }

            let got_decision = execute_tools(
                &tool_calls,
                turn,
                &model,
                &ctx,
                &mut state,
                &mut llm_messages,
                &mut mind_verdict,
            )
            .await;

            if got_decision {
                break;
            }

            if !(expects_response && (state.responded || state.attempted_send)) {
                inject_pending_messages(&ctx, &mut state, &mut llm_messages).await;
            }

            if matches!(collected.finish, FinishReason::Stop | FinishReason::Length) {
                break;
            }
        }

        if cancelled
            || state.responded
            || state.attempted_send
            || !expects_response
            || attempt >= max_attempts
        {
            break;
        }

        warn!(
            action = %ctx.action_id, attempt,
            max = max_attempts,
            "action did not call send_message, retrying"
        );
    }

    if let Some(guard) = composing_guard.as_mut() {
        let should_release = match &ctx.kind {
            SessionKind::Mind => true,
            SessionKind::Action(_) => !state.composing_released,
        };
        if should_release {
            guard.release().await;
        } else {
            guard.disarm();
        }
    }

    build_result(ctx, state, mind_verdict, attempt, cancelled)
}

fn session_cancellation_token(ctx: &SessionContext) -> super::action::CancellationToken {
    ctx.progress
        .read()
        .map(|progress| progress.cancellation_token())
        .unwrap_or_else(|_| super::action::RunningState::new().cancellation_token())
}

struct ComposingGuard {
    gateway: Arc<GatewayRouter>,
    target: Option<(String, String)>,
}

impl ComposingGuard {
    fn new(gateway: Arc<GatewayRouter>, gateway_id: String, external_id: String) -> Self {
        Self {
            gateway,
            target: Some((gateway_id, external_id)),
        }
    }

    async fn release(&mut self) {
        if let Some((gateway_id, external_id)) = self.target.take() {
            self.gateway
                .release_composing(&gateway_id, &external_id)
                .await;
        }
    }

    fn disarm(&mut self) {
        self.target = None;
    }
}

impl Drop for ComposingGuard {
    fn drop(&mut self) {
        let Some((gateway_id, external_id)) = self.target.take() else {
            return;
        };
        let gateway = self.gateway.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                gateway.release_composing(&gateway_id, &external_id).await;
            });
        }
    }
}

fn prompt_hash(messages: &[Message]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for message in messages {
        match message {
            Message::System(text) => {
                "system".hash(&mut hasher);
                text.hash(&mut hasher);
            }
            Message::User(user) => {
                "user".hash(&mut hasher);
                user.display_text().hash(&mut hasher);
            }
            Message::Assistant(assistant) => {
                "assistant".hash(&mut hasher);
                assistant.text.hash(&mut hasher);
                assistant.reasoning_content.hash(&mut hasher);
                for call in &assistant.tool_calls {
                    call.id.hash(&mut hasher);
                    call.name.hash(&mut hasher);
                    call.arguments.to_string().hash(&mut hasher);
                }
            }
            Message::Tool(result) => {
                "tool".hash(&mut hasher);
                result.call_id.hash(&mut hasher);
                result.content.hash(&mut hasher);
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

fn prompt_snapshot_messages(messages: &[Message]) -> Value {
    Value::Array(messages.iter().map(prompt_snapshot_message).collect())
}

fn prompt_snapshot_message(message: &Message) -> Value {
    match message {
        Message::System(text) => json!({
            "role": "system",
            "content": "[redacted]",
            "content_len": text.len(),
        }),
        Message::User(user) => prompt_snapshot_user_message(user),
        Message::Assistant(assistant) => {
            let tool_calls = assistant
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id.as_str(),
                        "name": call.name.as_str(),
                        "arguments": redact_prompt_trace_value(&call.arguments),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "role": "assistant",
                "content": assistant.text.as_ref().map(|_| "[redacted]"),
                "content_len": assistant.text.as_ref().map(|text| text.len()).unwrap_or(0),
                "reasoning_len": assistant
                    .reasoning_content
                    .as_deref()
                    .map(str::len)
                    .unwrap_or(0),
                "tool_calls": tool_calls,
            })
        }
        Message::Tool(result) => json!({
            "role": "tool",
            "call_id": result.call_id.as_str(),
            "content": prompt_snapshot_tool_content(&result.content),
        }),
    }
}

fn prompt_snapshot_user_message(user: &UserMessage) -> Value {
    match user {
        UserMessage::Text(text) => json!({
            "role": "user",
            "content": "[redacted]",
            "content_len": text.len(),
        }),
        UserMessage::Content(parts) => {
            let content_parts = parts
                .iter()
                .map(prompt_snapshot_content_part)
                .collect::<Vec<_>>();
            json!({
                "role": "user",
                "content": "[redacted]",
                "content_len": user.display_text().len(),
                "content_parts": content_parts,
            })
        }
    }
}

fn prompt_snapshot_content_part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text(text) => json!({
            "type": "text",
            "content": "[redacted]",
            "content_len": text.len(),
        }),
        ContentPart::ImageUrl(url) => json!({
            "type": "image_url",
            "url": redact_prompt_image_url(url),
        }),
    }
}

fn prompt_snapshot_tool_content(content: &str) -> Value {
    match serde_json::from_str::<Value>(content) {
        Ok(parsed) if parsed.is_object() || parsed.is_array() => redact_prompt_trace_value(&parsed),
        _ => Value::String(content.to_string()),
    }
}

fn redact_prompt_image_url(url: &str) -> &'static str {
    if url.starts_with("data:") {
        "[inline image redacted]"
    } else {
        "[image url redacted]"
    }
}

fn redact_prompt_trace_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if should_redact_prompt_trace_key(key) {
                        Value::String("[redacted]".into())
                    } else {
                        redact_prompt_trace_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_prompt_trace_value).collect()),
        Value::String(text) => {
            let trimmed = text.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                match serde_json::from_str::<Value>(text) {
                    Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                        Value::String(redact_prompt_trace_value(&parsed).to_string())
                    }
                    _ => value.clone(),
                }
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}

fn should_redact_prompt_trace_key(key: &str) -> bool {
    matches!(
        key,
        "content"
            | "text"
            | "summary"
            | "comm_style"
            | "evidence_quote"
            | "reason"
            | "task"
            | "external_id"
            | "sender_external_id"
            | "reply_external_id"
            | "source_message_id"
            | "media_url"
            | "url"
            | "raw_arguments"
    )
}

fn finish_reason_name(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
    }
}

fn build_result(
    mut ctx: SessionContext,
    mut state: SessionState,
    verdict: Option<MindVerdict>,
    attempts: usize,
    cancelled: bool,
) -> SessionResult {
    match ctx.kind {
        SessionKind::Mind => {
            if cancelled {
                SessionResult::Mind(MindVerdict::Drop)
            } else {
                SessionResult::Mind(verdict.unwrap_or(MindVerdict::Respond {
                    style_directive: None,
                }))
            }
        }
        SessionKind::Action(_) => {
            while let Ok(msg) = ctx.inject_rx.try_recv() {
                remember_injected_message(&mut state, msg);
            }
            let pending = std::mem::take(&mut state.pending_injected_messages);
            let delta = if !cancelled && tools::has_changes(&state.delta) {
                Some(state.delta)
            } else {
                None
            };
            SessionResult::Action(Outcome {
                responded: !cancelled && state.responded,
                attempted_send: state.attempted_send,
                cancelled,
                delta,
                pending_messages: pending,
                review_messages: state.presented_injected_messages,
                thoughts: state.thoughts,
                memories_formed: state.memories_formed,
                recalled_memory_ids: state.recalled_memory_ids,
                had_injections: state.presented_injection_count > 0,
                attempts: attempts as u32,
            })
        }
    }
}
