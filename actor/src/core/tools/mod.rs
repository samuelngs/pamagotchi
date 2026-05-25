mod context;
pub mod decision;
mod intent;
mod memory;
mod messaging;
mod person;
pub mod permission;
mod reflection;
mod time;
pub mod util;

pub use context::{SessionContext, SessionKind, SessionState, ToolOutcome};
pub use permission::check as check_permission;
pub use util::{empty_delta, has_changes};

use super::action::ActionKind;
use inference::Tool;
use serde_json::Value;

pub fn mind_tools() -> Vec<Tool> {
    let mut tools = decision::tools();
    tools.extend(memory::tools().into_iter().filter(|t| t.name == "recall_memories"));
    tools.extend(time::tools().into_iter().filter(|t| t.name == "get_current_time"));
    tools
}

pub fn action_tools(_kind: &ActionKind) -> Vec<Tool> {
    let mut tools = Vec::new();
    tools.extend(memory::tools());
    tools.extend(messaging::tools());
    tools.extend(person::tools());
    tools.extend(reflection::tools());
    tools.extend(intent::tools());
    tools.extend(time::tools());
    tools
}

pub async fn execute(
    name: &str,
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> ToolOutcome {
    if let Some(verdict) = decision::execute(name, args) {
        return ToolOutcome::Decision(verdict);
    }

    let result = match name {
        "recall_memories" => memory::recall(args, ctx).await,
        "form_memory" => memory::form(args, ctx, state).await,
        "forget_memory" => memory::forget(args, ctx).await,
        "send_message" => messaging::send(args, ctx, state).await,
        "lookup_contacts" => messaging::lookup_contacts(args, ctx).await,
        "read_messages" => messaging::read(args, ctx).await,
        "update_person" => person::update(args, ctx).await,
        "get_person" => person::get(args, ctx).await,
        "reflect" => reflection::reflect(args, ctx, state).await,
        "note_thought" => reflection::note_thought(args, ctx, state).await,
        "create_intent" => intent::create(args).await,
        "update_intent" => intent::update(args).await,
        "delete_intent" => intent::delete(args).await,
        "get_current_time" => time::current_time(args),
        "get_session_elapsed" => time::session_elapsed(ctx),
        _ => format!("Unknown tool: {name}"),
    };

    ToolOutcome::Result(result)
}
