use super::super::decision::MindVerdict;
use super::super::tools::{self, SessionContext, SessionKind, SessionState, ToolOutcome};
use async_trait::async_trait;
use inference::{
    AppServerToolCall, AppServerToolResult, AppServerToolRuntime, Capability, ChatRequest,
    FinishReason, InferenceProtocol, Message, RouteContext, StreamEvent,
};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

pub(super) struct Collected {
    pub(super) text: String,
    pub(super) reasoning: String,
    pub(super) partial_tools: Vec<PartialToolCall>,
    pub(super) finish: FinishReason,
    pub(super) app_server_decision: Option<MindVerdict>,
}

pub(super) struct OpenedStream {
    pub(super) stream: inference::ChatStream,
    app_server_tools: Option<mpsc::Receiver<AppServerToolRequest>>,
}

pub(super) async fn collect_stream(
    opened: &mut OpenedStream,
    ctx: &mut SessionContext,
    state: &mut SessionState,
) -> Collected {
    let mut c = Collected {
        text: String::new(),
        reasoning: String::new(),
        partial_tools: vec![],
        finish: FinishReason::Stop,
        app_server_decision: None,
    };
    loop {
        let event = tokio::select! {
            biased;
            request = recv_app_server_tool(opened.app_server_tools.as_mut()) => {
                match request {
                    Some(request) => {
                        if let Some(decision) = handle_app_server_tool(request, ctx, state).await {
                            c.app_server_decision = Some(decision);
                            c.finish = FinishReason::ToolCalls;
                            break;
                        }
                        continue;
                    }
                    None => {
                        opened.app_server_tools = None;
                        continue;
                    }
                }
            }
            event = opened.stream.recv() => event,
        };

        let Some(event) = event else {
            break;
        };
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
) -> Option<OpenedStream> {
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

        let stream_result = match &ep.protocol {
            InferenceProtocol::OpenAiCompatible(provider) => provider
                .chat_stream(&request)
                .await
                .map(|stream| (stream, None)),
            InferenceProtocol::CodexAppServer(provider) => {
                let (tool_tx, tool_rx) = mpsc::channel(16);
                provider
                    .run_turn(&request, Arc::new(SessionAppServerToolRuntime { tool_tx }))
                    .await
                    .map(|stream| (stream, Some(tool_rx)))
            }
        };

        match stream_result {
            Ok((stream, app_server_tools)) => {
                info!(action = %ctx.action_id, model = %ep.model, "LLM stream opened");
                return Some(OpenedStream {
                    stream,
                    app_server_tools,
                });
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

struct AppServerToolRequest {
    call: AppServerToolCall,
    response_tx: oneshot::Sender<AppServerToolResult>,
}

struct SessionAppServerToolRuntime {
    tool_tx: mpsc::Sender<AppServerToolRequest>,
}

#[async_trait]
impl AppServerToolRuntime for SessionAppServerToolRuntime {
    async fn call_tool(&self, call: AppServerToolCall) -> anyhow::Result<AppServerToolResult> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tool_tx
            .send(AppServerToolRequest { call, response_tx })
            .await?;
        Ok(response_rx.await?)
    }
}

async fn recv_app_server_tool(
    rx: Option<&mut mpsc::Receiver<AppServerToolRequest>>,
) -> Option<AppServerToolRequest> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn handle_app_server_tool(
    request: AppServerToolRequest,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> Option<MindVerdict> {
    let call = request.call;
    let args_short = truncate(&call.arguments.to_string(), 200);
    info!(action = %ctx.action_id, tool = %call.name, args = %args_short, "executing app-server tool");

    let (result, decision) =
        if let Err(denied) = tools::check_permission(&call.name, &call.arguments, ctx).await {
            (AppServerToolResult::error(denied), None)
        } else {
            match tools::execute(&call.name, &call.arguments, ctx, state).await {
                ToolOutcome::Result(result) => {
                    update_progress(&ctx.progress, state, &call.name);
                    (AppServerToolResult::text(result), None)
                }
                ToolOutcome::Decision(verdict) => (
                    AppServerToolResult::text(format!("{verdict:?}")),
                    Some(verdict),
                ),
            }
        };

    let _ = request.response_tx.send(result);
    decision
}

fn update_progress(
    progress: &std::sync::Arc<std::sync::RwLock<super::super::action::RunningState>>,
    state: &SessionState,
    tool_name: &str,
) {
    if let Ok(mut p) = progress.write() {
        p.responded = state.responded;
        p.last_tool = tool_name.to_string();
    }
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
