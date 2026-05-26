mod messages;
mod stream;
mod tool_calls;

#[cfg(test)]
mod tests;

use super::action::Outcome;
use super::decision::MindVerdict;
use super::tools::{self, SessionContext, SessionKind, SessionState};
use inference::{AssistantMessage, FinishReason, Message, RouteContext};
use messages::{
    build_prompt, ingest_messages, inject_pending_messages, required_capabilities,
    resolve_composing_target, user_message_for_inbound,
};
use protocol::{ConversationId, PersonId};
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
    let composing_target = resolve_composing_target(&ctx);
    if let Some((ref pid, ref eid)) = composing_target {
        ctx.gateway.acquire_composing(pid, eid).await;
    }

    let expects_response = matches!(&ctx.kind, SessionKind::Action(k) if k.expects_response());
    let max_attempts = if expects_response {
        ctx.max_action_attempts
    } else {
        1
    };
    let escalate_after = ctx.escalate_after;

    let mut attempt = 0;
    let mut mind_verdict: Option<MindVerdict> = None;
    let mut state = SessionState {
        responded: false,
        composing_released: false,
        delta: tools::empty_delta(ctx.messages.first().and_then(|m| m.person.clone())),
        thoughts: vec![],
        memories_formed: vec![],
        injected_messages: vec![],
    };

    loop {
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
        tracing::debug!(action = %ctx.action_id, system_prompt = %system_prompt, "system prompt content");

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
            log_turn_start(&ctx, turn, &llm_messages);

            let required_caps = required_capabilities(&ctx.messages, &state.injected_messages);
            let mut stream =
                match try_open_stream(&ctx, &llm_messages, &tool_defs, &required_caps).await {
                    Some(s) => s,
                    None => break,
                };

            let collected = collect_stream(&mut stream, &mut ctx, &mut state).await;
            info!(action = %ctx.action_id, turn, "LLM stream ended");

            let tool_calls = finalize_tool_calls(collected.partial_tools);

            info!(
                action = %ctx.action_id, turn,
                finish = ?collected.finish, tool_calls = tool_calls.len(),
                text_len = collected.text.len(), "LLM turn complete"
            );

            if !collected.text.is_empty() {
                info!(action = %ctx.action_id, thought = %collected.text, "internal monologue");
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

            let got_decision = execute_tools(
                &tool_calls,
                turn,
                &ctx,
                &mut state,
                &mut llm_messages,
                &mut mind_verdict,
            )
            .await;

            if got_decision {
                break;
            }

            inject_pending_messages(&ctx, &mut state, &mut llm_messages).await;

            if matches!(collected.finish, FinishReason::Stop | FinishReason::Length) {
                break;
            }
        }

        if state.responded || !expects_response || attempt >= max_attempts {
            break;
        }

        warn!(
            action = %ctx.action_id, attempt,
            max = max_attempts,
            "action did not call send_message, retrying"
        );
    }

    if let Some((ref pid, ref eid)) = composing_target {
        let should_release = match &ctx.kind {
            SessionKind::Mind => true,
            SessionKind::Action(_) => !state.composing_released,
        };
        if should_release {
            ctx.gateway.release_composing(pid, eid).await;
        }
    }

    build_result(ctx, state, mind_verdict)
}

fn build_result(
    mut ctx: SessionContext,
    state: SessionState,
    verdict: Option<MindVerdict>,
) -> SessionResult {
    match ctx.kind {
        SessionKind::Mind => SessionResult::Mind(verdict.unwrap_or(MindVerdict::Respond {
            style_directive: None,
        })),
        SessionKind::Action(_) => {
            let mut pending = vec![];
            while let Ok(msg) = ctx.inject_rx.try_recv() {
                pending.push(msg);
            }
            SessionResult::Action(Outcome {
                responded: state.responded,
                delta: if tools::has_changes(&state.delta) {
                    Some(state.delta)
                } else {
                    None
                },
                pending_messages: pending,
                had_injections: !state.injected_messages.is_empty(),
            })
        }
    }
}
