use super::action::Outcome;
use super::decision::MindVerdict;
use super::prompt;
use super::tools::{
    self, SessionContext, SessionKind, SessionState, ToolOutcome,
};
use inference::{
    AssistantMessage, ChatRequest, FinishReason, Message, StreamEvent, ToolCall,
};
use crate::store::{MessageRole, StoredMessage};
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

    let system_prompt = match build_prompt(&ctx).await {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, action = %ctx.action_id, "failed to build prompt");
            if let Some((ref pid, ref eid)) = composing_target {
                ctx.gateway.release_composing(pid, eid).await;
            }
            return default_result(&ctx.kind);
        }
    };

    info!(action = %ctx.action_id, system_prompt_len = system_prompt.len(), "system prompt built");
    tracing::debug!(action = %ctx.action_id, system_prompt = %system_prompt, "system prompt content");

    let mut llm_messages = vec![Message::system(system_prompt)];
    ingest_messages(&ctx, &mut llm_messages).await;

    let tool_defs = match &ctx.kind {
        SessionKind::Mind => tools::mind_tools(),
        SessionKind::Action(kind) => tools::action_tools(kind),
    };

    let mut state = SessionState {
        responded: false,
        composing_released: false,
        delta: tools::empty_delta(ctx.messages.first().and_then(|m| m.person.clone())),
        thoughts: vec![],
        memories_formed: vec![],
        injected_messages: vec![],
    };

    let mut mind_verdict: Option<MindVerdict> = None;

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

        if tool_calls.is_empty() { break; }

        llm_messages.push(Message::Assistant(AssistantMessage {
            text: if collected.text.is_empty() { None } else { Some(collected.text) },
            reasoning_content: if collected.reasoning.is_empty() { None } else { Some(collected.reasoning) },
            tool_calls: tool_calls.clone(),
        }));

        let got_decision = execute_tools(
            &tool_calls, turn, &ctx, &mut state, &mut llm_messages, &mut mind_verdict,
        ).await;

        if got_decision { break; }

        inject_pending_messages(&ctx, &mut state, &mut llm_messages).await;

        if matches!(collected.finish, FinishReason::Stop | FinishReason::Length) { break; }
    }

    cleanup_composing(&ctx, &composing_target, &mind_verdict, &state).await;
    build_result(ctx, state, mind_verdict)
}

async fn build_prompt(ctx: &SessionContext) -> anyhow::Result<String> {
    prompt::build_system_prompt(
        &ctx.state, &ctx.store, &ctx.kind, &ctx.messages,
        ctx.conversation.as_ref(), ctx,
        &ctx.authority,
    ).await
}

async fn ingest_messages(ctx: &SessionContext, llm_messages: &mut Vec<Message>) {
    for inbound in &ctx.messages {
        let display = inbound.display_content();
        llm_messages.push(Message::user(&display));
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: inbound.timestamp, role: MessageRole::User,
                content: display, person: inbound.person.clone(), metadata: inbound.metadata.clone(),
            };
            ctx.store.append_message(conv, None, None, &stored).await.ok();
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
        text: String::new(), reasoning: String::new(),
        partial_tools: vec![], finish: FinishReason::Stop,
    };
    while let Some(event) = stream.recv().await {
        let event = match event {
            Ok(e) => e,
            Err(e) => { warn!(%e, action = %ctx.action_id, "stream event error"); break; }
        };
        match event {
            StreamEvent::TextDelta(d) => c.text.push_str(&d),
            StreamEvent::ReasoningDelta(d) => c.reasoning.push_str(&d),
            StreamEvent::ToolCallBegin { index, id, name } => {
                if c.partial_tools.len() <= index {
                    c.partial_tools.resize_with(index + 1, PartialToolCall::default);
                }
                if !id.is_empty() { c.partial_tools[index].id = id; }
                if !name.is_empty() { c.partial_tools[index].name = name; }
            }
            StreamEvent::ToolCallDelta { index, arguments_delta } => {
                if c.partial_tools.len() <= index {
                    c.partial_tools.resize_with(index + 1, PartialToolCall::default);
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
    partials.into_iter().map(|tc| ToolCall {
        id: tc.id, name: tc.name,
        arguments: serde_json::from_str(&tc.arguments)
            .unwrap_or(Value::Object(Default::default())),
    }).collect()
}

async fn execute_tools(
    tool_calls: &[ToolCall], turn: usize,
    ctx: &SessionContext, state: &mut SessionState,
    llm_messages: &mut Vec<Message>, mind_verdict: &mut Option<MindVerdict>,
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
    ctx: &SessionContext, state: &mut SessionState, llm_messages: &mut Vec<Message>,
) {
    for msg in &state.injected_messages {
        let display = msg.display_content();
        if llm_messages.iter().any(|m| matches!(m, Message::User(u) if *u == display)) { continue; }
        llm_messages.push(Message::system("--- New message arrived while you were working. Address it before finishing. ---"));
        llm_messages.push(Message::user(&display));
        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: msg.timestamp, role: MessageRole::User,
                content: display, person: msg.person.clone(), metadata: msg.metadata.clone(),
            };
            ctx.store.append_message(conv, None, None, &stored).await.ok();
        }
    }
}

async fn cleanup_composing(
    ctx: &SessionContext, target: &Option<(String, String)>,
    verdict: &Option<MindVerdict>, state: &SessionState,
) {
    match &ctx.kind {
        SessionKind::Mind => {
            if !matches!(verdict, Some(MindVerdict::Respond { .. })) {
                if let Some((pid, eid)) = target { ctx.gateway.release_composing(pid, eid).await; }
            }
        }
        SessionKind::Action(_) => {
            if !state.composing_released {
                if let Some((pid, eid)) = target { ctx.gateway.release_composing(pid, eid).await; }
            }
        }
    }
}

fn build_result(mut ctx: SessionContext, state: SessionState, verdict: Option<MindVerdict>) -> SessionResult {
    match ctx.kind {
        SessionKind::Mind => SessionResult::Mind(verdict.unwrap_or(MindVerdict::Respond { style_directive: None })),
        SessionKind::Action(_) => {
            let mut pending = vec![];
            while let Ok(msg) = ctx.inject_rx.try_recv() { pending.push(msg); }
            SessionResult::Action(Outcome {
                responded: state.responded,
                delta: if tools::has_changes(&state.delta) { Some(state.delta) } else { None },
                pending_messages: pending,
                had_injections: !state.injected_messages.is_empty(),
            })
        }
    }
}

fn default_result(kind: &SessionKind) -> SessionResult {
    match kind {
        SessionKind::Mind => SessionResult::Mind(MindVerdict::Respond { style_directive: None }),
        SessionKind::Action(_) => SessionResult::Action(Outcome {
            responded: false,
            delta: None,
            pending_messages: vec![],
            had_injections: false,
        }),
    }
}

fn resolve_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    ctx.messages.first().map(|msg| (msg.gateway_id.clone(), msg.external_id.clone()))
}

fn log_turn_start(ctx: &SessionContext, turn: usize, msgs: &[Message]) {
    let summary: Vec<String> = msgs.iter().map(|m| match m {
        Message::System(s) => format!("system({})", s.len()),
        Message::User(s) => format!("user({})", s.len()),
        Message::Assistant(a) => format!("assistant(text={},tools={})", a.text.as_ref().map_or(0, |t| t.len()), a.tool_calls.len()),
        Message::Tool(t) => format!("tool_result({}:{})", t.call_id.chars().take(8).collect::<String>(), t.content.len()),
    }).collect();
    info!(action = %ctx.action_id, turn, messages = ?summary, "LLM request starting");
}

fn update_progress(
    progress: &std::sync::Arc<std::sync::RwLock<super::action::RunningState>>,
    state: &SessionState, tool_name: &str,
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
    if s.len() > max { format!("{}...", &s[..max]) } else { s.to_string() }
}

#[derive(Default)]
struct PartialToolCall { id: String, name: String, arguments: String }
