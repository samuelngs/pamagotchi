use super::super::decision::MindVerdict;
use super::super::tools::{self, SessionContext, SessionState, ToolOutcome};
use super::stream::{PartialToolCall, truncate};
use inference::{Message, ToolCall};
use serde_json::Value;
use tracing::info;

pub(super) fn finalize_tool_calls(partials: Vec<PartialToolCall>) -> Vec<ToolCall> {
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

pub(super) async fn execute_tools(
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
