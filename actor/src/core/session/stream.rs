use super::super::decision::MindVerdict;
use super::super::tools::{self, SessionContext, SessionKind, SessionState, ToolOutcome};
use super::messages::remember_injected_message;
use async_trait::async_trait;
use inference::{
    AppServerToolCall, AppServerToolResult, AppServerToolRuntime, Capability, ChatRequest,
    FinishReason, InferenceProtocol, Message, RouteContext, StreamEvent,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

pub(super) struct Collected {
    pub(super) text: String,
    pub(super) reasoning: String,
    pub(super) partial_tools: Vec<PartialToolCall>,
    pub(super) finish: FinishReason,
    pub(super) input_tokens: Option<u32>,
    pub(super) output_tokens: Option<u32>,
    pub(super) app_server_decision: Option<MindVerdict>,
}

pub(super) struct OpenedStream {
    pub(super) stream: inference::ChatStream,
    pub(super) model: String,
    app_server_tools: Option<mpsc::Receiver<AppServerToolRequest>>,
}

pub(super) async fn collect_stream(
    opened: &mut OpenedStream,
    ctx: &mut SessionContext,
    state: &mut SessionState,
    turn: usize,
) -> Collected {
    let mut c = Collected {
        text: String::new(),
        reasoning: String::new(),
        partial_tools: vec![],
        finish: FinishReason::Stop,
        input_tokens: None,
        output_tokens: None,
        app_server_decision: None,
    };
    loop {
        let event = tokio::select! {
            biased;
            request = recv_app_server_tool(opened.app_server_tools.as_mut()) => {
                match request {
                    Some(request) => {
                        if let Some(decision) = handle_app_server_tool(request, ctx, state, turn).await {
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
            StreamEvent::Usage(usage) => {
                c.input_tokens = Some(usage.input_tokens);
                c.output_tokens = Some(usage.output_tokens);
            }
        }
        while let Ok(msg) = ctx.inject_rx.try_recv() {
            if remember_injected_message(state, msg) {
                info!(action = %ctx.action_id, "received injected message mid-stream");
            } else {
                ctx.metrics.record_duplicate_message_suppression();
                info!(action = %ctx.action_id, "suppressed duplicate injected message mid-stream");
            }
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
                    model: ep.model.clone(),
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
    turn: usize,
) -> Option<MindVerdict> {
    let call = request.call;
    let args_short = truncate(&call.arguments.to_string(), 200);
    info!(action = %ctx.action_id, tool = %call.name, args = %args_short, "executing app-server tool");
    let latency_start = Instant::now();
    let started_at = super::super::tools::util::now();

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
    let success = result.success;
    let result_content = result
        .content
        .iter()
        .map(|content| match content {
            inference::AppServerToolResultContent::Text(text) => {
                serde_json::json!({"type": "text", "text": text})
            }
            inference::AppServerToolResultContent::ImageUrl(url) => {
                serde_json::json!({"type": "image_url", "url": url})
            }
        })
        .collect::<Vec<_>>();
    let result_json = serde_json::json!({
        "content": result_content,
        "success": result.success,
    });
    let record = crate::store::ToolCallRecord {
        action_id: ctx.action_id.0.clone(),
        turn: turn as u32,
        call_id: call.id.clone(),
        name: call.name.clone(),
        args: call.arguments.clone(),
        result: result_json,
        success,
        started_at,
        ended_at: super::super::tools::util::now(),
    };
    if let Err(e) = ctx.store.append_tool_call(&record).await {
        warn!(action = %ctx.action_id, %e, "failed to persist app-server tool call");
    }
    ctx.metrics.record_tool_call(&call.name, success);
    ctx.metrics
        .record_app_server_tool_latency(latency_start.elapsed().as_millis() as u64);

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
    let mut chars = s.chars();
    let out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{out}...")
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

#[cfg(test)]
mod tests {
    use super::truncate;
    use serde_json::json;

    #[test]
    fn truncate_handles_multilingual_multibyte_cutoff_boundaries() {
        let cases = [
            ("abc回def", 4, "abc回..."),
            ("abéçd", 3, "abé..."),
            ("hi🙂there", 3, "hi🙂..."),
            ("abcمdef", 4, "abcم..."),
            ("abनमस्ते", 3, "abन..."),
        ];

        for (value, max, expected) in cases {
            assert!(
                !value.is_char_boundary(max),
                "{value:?} byte cutoff {max} should be inside a multibyte character"
            );
            assert_eq!(truncate(value, max), expected);
        }
    }

    #[test]
    fn truncate_keeps_untruncated_multibyte_text_without_ellipsis() {
        let value = "回家";

        assert_eq!(truncate(value, 2), value);
        assert_eq!(truncate(value, 3), value);
    }

    #[test]
    fn truncate_handles_apply_review_payload_with_traditional_chinese_summary() {
        let payload = (0..3)
            .find_map(|padding| {
                let summary = format!(
                    "{}{}",
                    "a".repeat(padding),
                    "回顧部署後續事項，保留繁體中文摘要。".repeat(20)
                );
                let payload = json!({
                    "conversation_summary": {
                        "conversation_id": "relay:local",
                        "summary": summary,
                        "covered_message_ids": ["msg-traditional-chinese"]
                    },
                    "memories": []
                })
                .to_string();
                (!payload.is_char_boundary(200)).then_some(payload)
            })
            .expect("test payload should place byte 200 inside a multibyte character");

        let truncated = truncate(&payload, 200);

        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
