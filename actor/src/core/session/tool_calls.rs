use super::super::decision::MindVerdict;
use super::super::tools::{self, SessionContext, SessionState, ToolOutcome};
use super::stream::{PartialToolCall, truncate};
use crate::store::ToolCallRecord;
use inference::{Message, ToolCall};
use serde_json::Value;
use tracing::{info, warn};

const INVALID_TOOL_JSON_KEY: &str = "__invalid_tool_json";

pub(super) fn finalize_tool_calls(partials: Vec<PartialToolCall>) -> Vec<ToolCall> {
    partials
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id,
            name: tc.name,
            arguments: match serde_json::from_str(&tc.arguments) {
                Ok(arguments) => arguments,
                Err(error) => invalid_tool_json(tc.arguments, error.to_string()),
            },
        })
        .collect()
}

pub(super) async fn execute_tools(
    tool_calls: &[ToolCall],
    turn: usize,
    model: &str,
    ctx: &SessionContext,
    state: &mut SessionState,
    llm_messages: &mut Vec<Message>,
    mind_verdict: &mut Option<MindVerdict>,
) -> bool {
    for tc in tool_calls {
        let args_short = truncate(&tc.arguments.to_string(), 200);
        info!(action = %ctx.action_id, turn, tool = %tc.name, args = %args_short, "executing tool");

        if let Some(error) = invalid_tool_json_error(&tc.arguments) {
            let now = tools::util::now();
            warn!(
                action = %ctx.action_id,
                turn,
                tool = %tc.name,
                error = %error,
                "tool arguments were invalid JSON; skipping execution"
            );
            llm_messages.push(Message::tool_result(
                &tc.id,
                &format!("Tool arguments were invalid JSON: {error}. No tool was executed."),
            ));
            let record = ToolCallRecord {
                action_id: ctx.action_id.0.clone(),
                turn: turn as u32,
                call_id: tc.id.clone(),
                name: tc.name.clone(),
                args: tc.arguments.clone(),
                result: serde_json::json!({
                    "error": error,
                    "executed": false,
                }),
                success: false,
                started_at: now,
                ended_at: now,
            };
            if let Err(e) = ctx.store.append_tool_call(&record).await {
                warn!(action = %ctx.action_id, %e, "failed to persist invalid tool call");
            }
            ctx.metrics.record_malformed_tool_json(model);
            ctx.metrics.record_tool_call(&tc.name, false);
            continue;
        }

        let started_at = tools::util::now();
        if let Err(denied) = tools::check_permission(&tc.name, &tc.arguments, ctx).await {
            info!(action = %ctx.action_id, tool = %tc.name, "tool denied: {denied}");
            llm_messages.push(Message::tool_result(&tc.id, &denied));
            let record = ToolCallRecord {
                action_id: ctx.action_id.0.clone(),
                turn: turn as u32,
                call_id: tc.id.clone(),
                name: tc.name.clone(),
                args: tc.arguments.clone(),
                result: serde_json::json!({
                    "error": denied,
                    "executed": false,
                }),
                success: false,
                started_at,
                ended_at: tools::util::now(),
            };
            if let Err(e) = ctx.store.append_tool_call(&record).await {
                warn!(action = %ctx.action_id, %e, "failed to persist denied tool call");
            }
            ctx.metrics.record_tool_call(&tc.name, false);
            continue;
        }

        match tools::execute(&tc.name, &tc.arguments, ctx, state).await {
            ToolOutcome::Result(result) => {
                info!(action = %ctx.action_id, turn, tool = %tc.name, result = %truncate(&result, 200), "tool completed");
                llm_messages.push(Message::tool_result(&tc.id, &result));
                let record = ToolCallRecord {
                    action_id: ctx.action_id.0.clone(),
                    turn: turn as u32,
                    call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    args: tc.arguments.clone(),
                    result: serde_json::json!({ "result": result }),
                    success: true,
                    started_at,
                    ended_at: tools::util::now(),
                };
                if let Err(e) = ctx.store.append_tool_call(&record).await {
                    warn!(action = %ctx.action_id, %e, "failed to persist tool call");
                }
                ctx.metrics.record_tool_call(&tc.name, true);
                update_progress(&ctx.progress, state, &tc.name);
            }
            ToolOutcome::Decision(verdict) => {
                let record = ToolCallRecord {
                    action_id: ctx.action_id.0.clone(),
                    turn: turn as u32,
                    call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    args: tc.arguments.clone(),
                    result: serde_json::json!({ "decision": format!("{verdict:?}") }),
                    success: true,
                    started_at,
                    ended_at: tools::util::now(),
                };
                if let Err(e) = ctx.store.append_tool_call(&record).await {
                    warn!(action = %ctx.action_id, %e, "failed to persist decision tool call");
                }
                ctx.metrics.record_tool_call(&tc.name, true);
                *mind_verdict = Some(verdict);
                return true;
            }
        }
    }
    false
}

fn invalid_tool_json(raw_arguments: String, error: String) -> Value {
    serde_json::json!({
        INVALID_TOOL_JSON_KEY: true,
        "raw_arguments": raw_arguments,
        "error": error,
    })
}

fn invalid_tool_json_error(args: &Value) -> Option<&str> {
    args.get(INVALID_TOOL_JSON_KEY)
        .and_then(Value::as_bool)
        .filter(|invalid| *invalid)
        .and_then(|_| args.get("error"))
        .and_then(Value::as_str)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_tool_arguments_are_preserved_as_error() {
        let calls = finalize_tool_calls(vec![PartialToolCall {
            id: "call-1".into(),
            name: "send_message".into(),
            arguments: "{\"content\":".into(),
        }]);

        assert_eq!(calls.len(), 1);
        let error = invalid_tool_json_error(&calls[0].arguments).expect("invalid json marker");
        assert!(error.contains("EOF") || error.contains("expected"));
        assert_eq!(calls[0].arguments["raw_arguments"], "{\"content\":");
    }
}
