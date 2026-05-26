use super::super::tools::{SessionContext, SessionKind, SessionState};
use inference::{Capability, ChatRequest, FinishReason, Message, RouteContext, StreamEvent};
use tracing::{info, warn};

pub(super) struct Collected {
    pub(super) text: String,
    pub(super) reasoning: String,
    pub(super) partial_tools: Vec<PartialToolCall>,
    pub(super) finish: FinishReason,
}

pub(super) async fn collect_stream(
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

pub(super) fn log_turn_start(ctx: &SessionContext, turn: usize, msgs: &[Message]) {
    let summary: Vec<String> = msgs
        .iter()
        .map(|m| match m {
            Message::System(s) => format!("system({})", s.len()),
            Message::User(s) => format!("user({})", s.display_text().len()),
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

pub(super) async fn try_open_stream(
    ctx: &SessionContext,
    llm_messages: &[Message],
    tool_defs: &[inference::Tool],
    required_caps: &[Capability],
) -> Option<inference::ChatStream> {
    let endpoints = if required_caps.is_empty() {
        ctx.endpoints.clone()
    } else {
        let resolved = ctx
            .router
            .resolve_chain_requiring(&route_context(ctx), required_caps);
        if resolved.is_empty() {
            warn!(
                action = %ctx.action_id,
                required = ?required_caps,
                "no inference endpoint satisfies required media capabilities; falling back to text context"
            );
            ctx.endpoints.clone()
        } else {
            resolved
        }
    };

    for (i, ep) in endpoints.iter().enumerate() {
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
                let remaining = endpoints.len() - i - 1;
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

fn route_context(ctx: &SessionContext) -> RouteContext {
    match &ctx.kind {
        SessionKind::Mind => RouteContext::Mind,
        SessionKind::Action(_) => RouteContext::Action(ctx.reasoning),
    }
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}

#[derive(Default)]
pub(super) struct PartialToolCall {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}
