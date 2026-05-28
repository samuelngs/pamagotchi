mod composing;
mod messages;
mod result;
mod snapshot;
mod stream;
mod tool_calls;

#[cfg(test)]
mod tests;

use super::action::Outcome;
use super::decision::MindVerdict;
use super::tools::{self, SessionContext, SessionKind, SessionState};
use crate::store::{ActionPromptSnapshotRecord, ActionTurnRecord};
use composing::{ComposingGuard, session_cancellation_token};
use gateway::GatewayRouter;
use inference::{AssistantMessage, ContentPart, FinishReason, Message, RouteContext, UserMessage};
use messages::{
    build_prompt, ingest_messages, inject_pending_messages, remember_injected_message,
    required_capabilities, resolve_composing_target, source_message_keys, user_message_for_inbound,
};
use protocol::{ConversationId, PersonId};
use result::{build_result, finish_reason_name};
use serde_json::{Value, json};
use snapshot::{prompt_hash, prompt_snapshot_messages};
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
