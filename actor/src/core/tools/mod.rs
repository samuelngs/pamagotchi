mod context;
pub mod decision;
mod intent;
mod memory;
mod messaging;
pub mod permission;
mod person;
mod reflection;
mod review;
mod time;
pub mod util;

pub use context::{
    SessionContext, SessionKind, SessionState, TYPING_ACTIVE_SECS, ToolOutcome, TypingState,
};
pub use permission::check as check_permission;
pub use util::{empty_delta, has_changes};

use super::action::ActionKind;
use inference::Tool;
use serde_json::Value;

pub fn mind_tools() -> Vec<Tool> {
    let mut tools = decision::tools();
    tools.extend(
        memory::tools()
            .into_iter()
            .filter(|t| t.name == "recall_memories"),
    );
    tools.extend(
        time::tools()
            .into_iter()
            .filter(|t| t.name == "get_current_time"),
    );
    tools.extend(
        messaging::tools()
            .into_iter()
            .filter(|t| t.name == "read_messages"),
    );
    tools
}

pub fn action_tools(kind: &ActionKind) -> Vec<Tool> {
    let mut tools = Vec::new();
    tools.extend(memory::tools());
    tools.extend(messaging_tools_for(kind));
    tools.extend(person::tools());
    tools.extend(reflection::tools());
    if matches!(kind, ActionKind::Review) {
        tools.extend(review::tools());
    }
    tools.extend(intent::tools());
    tools.extend(time::tools());
    tools
}

fn messaging_tools_for(kind: &ActionKind) -> Vec<Tool> {
    let tools = messaging::tools();
    if !kind.expects_response() {
        tools
            .into_iter()
            .filter(|tool| tool.name != "send_message")
            .collect()
    } else {
        tools
    }
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
        "recall_memories" => memory::recall(args, ctx, state).await,
        "form_memory" => memory::form(args, ctx, state).await,
        "inspect_memory" => memory::inspect(args, ctx).await,
        "promote_profile_memory_to_person" => {
            memory::promote_profile_memory_to_person(args, ctx).await
        }
        "demote_person_memory_to_profile" => {
            memory::demote_person_memory_to_profile(args, ctx).await
        }
        "forget_memory" => memory::forget(args, ctx).await,
        "delete_memory" => memory::delete(args, ctx).await,
        "send_message" => messaging::send(args, ctx, state).await,
        "read_messages" => messaging::read_with_state(args, ctx, state).await,
        "update_conversation_summary" => messaging::update_conversation_summary(args, ctx).await,
        "update_profile" => person::update_profile(args, ctx).await,
        "update_person" => person::update(args, ctx).await,
        "get_person" => person::get(args, ctx).await,
        "request_identity_verification" => person::request_identity_verification(args, ctx).await,
        "resolve_identity_verification" => person::resolve_identity_verification(args, ctx).await,
        "detach_profile_from_person" => person::detach_profile(args, ctx).await,
        "reject_profile_person_link" => person::reject_profile_person_link(args, ctx).await,
        "upsert_social_relation" => person::upsert_social_relation(args, ctx, state).await,
        "reflect" => reflection::reflect(args, ctx, state).await,
        "note_thought" => reflection::note_thought(args, ctx, state).await,
        "apply_review" => review::apply(args, ctx, state).await,
        "create_intent" => intent::create(args, ctx).await,
        "update_intent" => intent::update(args, ctx).await,
        "delete_intent" => intent::delete(args, ctx).await,
        "get_current_time" => time::current_time(args),
        "get_session_elapsed" => time::session_elapsed(ctx),
        _ => format!("Unknown tool: {name}"),
    };

    ToolOutcome::Result(result)
}

#[cfg(test)]
mod tests;
