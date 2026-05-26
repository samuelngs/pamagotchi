use super::action::Outcome;
use super::decision::MindVerdict;
use super::prompt;
use super::tools::{self, SessionContext, SessionKind, SessionState, ToolOutcome};
use crate::store::{MessageRole, StoredMessage};
use inference::{
    AssistantMessage, ChatRequest, FinishReason, Message, RouteContext, StreamEvent, ToolCall,
};
use protocol::{ConversationId, PersonId};
use serde_json::Value;
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
        if attempt == 1 {
            ingest_messages(&ctx, &mut llm_messages).await;
        } else {
            for inbound in &ctx.messages {
                llm_messages.push(Message::user(&inbound.display_content()));
            }
        }

        let tool_defs = match &ctx.kind {
            SessionKind::Mind => tools::mind_tools(),
            SessionKind::Action(kind) => tools::action_tools(kind),
        };

        state.responded = false;

        for turn in 0..ctx.max_turns {
            log_turn_start(&ctx, turn, &llm_messages);

            let mut stream = match try_open_stream(&ctx, &llm_messages, &tool_defs).await {
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

async fn build_prompt(ctx: &SessionContext) -> anyhow::Result<String> {
    prompt::build_system_prompt(
        &ctx.state,
        &ctx.store,
        &ctx.kind,
        &ctx.messages,
        ctx.conversation.as_ref(),
        ctx,
        &ctx.authority,
    )
    .await
}

async fn ingest_messages(ctx: &SessionContext, llm_messages: &mut Vec<Message>) {
    for inbound in &ctx.messages {
        let display = inbound.display_content();
        llm_messages.push(Message::user(&display));
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: inbound.timestamp,
                role: MessageRole::User,
                content: display,
                identity: inbound.identity.clone(),
                profile: inbound.profile.clone(),
                person: inbound.person.clone(),
                metadata: message_metadata(inbound),
            };
            ctx.store
                .append_message(conv, None, None, &stored)
                .await
                .ok();
        }
    }
}

struct Collected {
    text: String,
    reasoning: String,
    partial_tools: Vec<PartialToolCall>,
    finish: FinishReason,
}

async fn collect_stream(
    stream: &mut inference::ChatStream,
    ctx: &mut SessionContext,
    state: &mut SessionState,
) -> Collected {
    let mut c = Collected {
        text: String::new(),
        reasoning: String::new(),
        partial_tools: vec![],
        finish: FinishReason::Stop,
    };
    while let Some(event) = stream.recv().await {
        let event = match event {
            Ok(e) => e,
            Err(e) => {
                warn!(%e, action = %ctx.action_id, "stream event error");
                break;
            }
        };
        match event {
            StreamEvent::TextDelta(d) => c.text.push_str(&d),
            StreamEvent::ReasoningDelta(d) => c.reasoning.push_str(&d),
            StreamEvent::ToolCallBegin { index, id, name } => {
                if c.partial_tools.len() <= index {
                    c.partial_tools
                        .resize_with(index + 1, PartialToolCall::default);
                }
                if !id.is_empty() {
                    c.partial_tools[index].id = id;
                }
                if !name.is_empty() {
                    c.partial_tools[index].name = name;
                }
            }
            StreamEvent::ToolCallDelta {
                index,
                arguments_delta,
            } => {
                if c.partial_tools.len() <= index {
                    c.partial_tools
                        .resize_with(index + 1, PartialToolCall::default);
                }
                c.partial_tools[index].arguments.push_str(&arguments_delta);
            }
            StreamEvent::FinishReason(r) => c.finish = r,
            StreamEvent::Usage(_) => {}
        }
        while let Ok(msg) = ctx.inject_rx.try_recv() {
            info!(action = %ctx.action_id, "received injected message mid-stream");
            state.injected_messages.push(msg);
        }
    }
    c
}

fn finalize_tool_calls(partials: Vec<PartialToolCall>) -> Vec<ToolCall> {
    partials
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.name,
            arguments: serde_json::from_str(&tc.arguments)
                .unwrap_or(Value::Object(Default::default())),
        })
        .collect()
}

async fn execute_tools(
    tool_calls: &[ToolCall],
    turn: usize,
    ctx: &SessionContext,
    state: &mut SessionState,
    llm_messages: &mut Vec<Message>,
    mind_verdict: &mut Option<MindVerdict>,
) -> bool {
    for tc in tool_calls {
        let args_short = truncate(&tc.arguments.to_string(), 200);
        info!(action = %ctx.action_id, turn, tool = %tc.name, args = %args_short, "executing tool");

        if let Err(denied) = tools::check_permission(&tc.name, &tc.arguments, ctx).await {
            info!(action = %ctx.action_id, tool = %tc.name, "tool denied: {denied}");
            llm_messages.push(Message::tool_result(&tc.id, &denied));
            continue;
        }

        match tools::execute(&tc.name, &tc.arguments, ctx, state).await {
            ToolOutcome::Result(result) => {
                info!(action = %ctx.action_id, turn, tool = %tc.name, result = %truncate(&result, 200), "tool completed");
                llm_messages.push(Message::tool_result(&tc.id, &result));
                update_progress(&ctx.progress, state, &tc.name);
            }
            ToolOutcome::Decision(verdict) => {
                *mind_verdict = Some(verdict);
                return true;
            }
        }
    }
    false
}

async fn inject_pending_messages(
    ctx: &SessionContext,
    state: &mut SessionState,
    llm_messages: &mut Vec<Message>,
) {
    for msg in &state.injected_messages {
        let display = msg.display_content();
        if llm_messages
            .iter()
            .any(|m| matches!(m, Message::User(u) if *u == display))
        {
            continue;
        }
        llm_messages.push(Message::system(
            "--- New message arrived while you were working. Address it before finishing. ---",
        ));
        llm_messages.push(Message::user(&display));
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: msg.timestamp,
                role: MessageRole::User,
                content: display,
                identity: msg.identity.clone(),
                profile: msg.profile.clone(),
                person: msg.person.clone(),
                metadata: message_metadata(msg),
            };
            ctx.store
                .append_message(conv, None, None, &stored)
                .await
                .ok();
        }
    }
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

fn resolve_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages.first().and_then(|msg| {
        if msg.gateway_id.is_empty() || msg.external_id.is_empty() {
            None
        } else {
            Some((msg.gateway_id.clone(), msg.external_id.clone()))
        }
    })
}

fn message_metadata(msg: &protocol::InboundMessage) -> Value {
    let mut metadata = msg.metadata.clone();
    if msg.attachments.is_empty() {
        return metadata;
    }

    let attachments_value = serde_json::to_value(&msg.attachments).unwrap_or(Value::Null);
    match &mut metadata {
        Value::Object(obj) => {
            obj.insert("attachments".into(), attachments_value);
            metadata
        }
        Value::Null => serde_json::json!({ "attachments": attachments_value }),
        other => serde_json::json!({
            "source_metadata": other.clone(),
            "attachments": attachments_value,
        }),
    }
}

fn log_turn_start(ctx: &SessionContext, turn: usize, msgs: &[Message]) {
    let summary: Vec<String> = msgs
        .iter()
        .map(|m| match m {
            Message::System(s) => format!("system({})", s.len()),
            Message::User(s) => format!("user({})", s.len()),
            Message::Assistant(a) => format!(
                "assistant(text={},tools={})",
                a.text.as_ref().map_or(0, |t| t.len()),
                a.tool_calls.len()
            ),
            Message::Tool(t) => format!(
                "tool_result({}:{})",
                t.call_id.chars().take(8).collect::<String>(),
                t.content.len()
            ),
        })
        .collect();
    info!(action = %ctx.action_id, turn, messages = ?summary, "LLM request starting");
}

fn update_progress(
    progress: &std::sync::Arc<std::sync::RwLock<super::action::RunningState>>,
    state: &SessionState,
    tool_name: &str,
) {
    if let Ok(mut p) = progress.write() {
        p.responded = state.responded;
        p.last_tool = tool_name.to_string();
    }
}

async fn try_open_stream(
    ctx: &SessionContext,
    llm_messages: &[Message],
    tool_defs: &[inference::Tool],
) -> Option<inference::ChatStream> {
    for (i, ep) in ctx.endpoints.iter().enumerate() {
        let request = ChatRequest::new(&ep.model, llm_messages.to_vec())
            .with_tools(tool_defs.to_vec())
            .with_tool_choice(inference::ToolChoice::Auto)
            .with_sampling(&ep.sampling);

        match ep.provider.chat_stream(&request).await {
            Ok(s) => {
                info!(action = %ctx.action_id, model = %ep.model, "LLM stream opened");
                return Some(s);
            }
            Err(e) => {
                let remaining = ctx.endpoints.len() - i - 1;
                if remaining > 0 {
                    warn!(%e, action = %ctx.action_id, model = %ep.model, remaining, "LLM failed, trying next endpoint");
                } else {
                    warn!(%e, action = %ctx.action_id, model = %ep.model, "LLM failed, no more endpoints");
                }
            }
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{InboundMessage, MediaAssetId, MediaAttachment, MediaKind};

    fn inbound(metadata: Value) -> InboundMessage {
        InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "whatsapp".into(),
            external_id: "chat-1".into(),
            conversation: ConversationId("whatsapp:chat-1".into()),
            group: None,
            identity: None,
            profile: None,
            person: None,
            content: String::new(),
            attachments: vec![MediaAttachment {
                kind: MediaKind::Sticker,
                asset_id: Some(MediaAssetId("media-1".into())),
                url: None,
                mime: Some("image/webp".into()),
                filename: Some("sticker.webp".into()),
                size: Some(99),
            }],
            timestamp: 1,
            metadata,
        }
    }

    #[test]
    fn message_metadata_embeds_attachments() {
        let metadata = message_metadata(&inbound(serde_json::json!({ "sender": "user" })));

        assert_eq!(metadata["sender"], "user");
        assert_eq!(metadata["attachments"][0]["kind"], "Sticker");
        assert_eq!(metadata["attachments"][0]["asset_id"], "media-1");
        assert_eq!(metadata["attachments"][0]["mime"], "image/webp");
    }
}
