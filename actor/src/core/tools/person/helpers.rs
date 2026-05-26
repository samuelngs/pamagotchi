use super::super::context::SessionContext;
use protocol::PersonId;
use serde_json::Value;

pub(super) fn resolve_person_ref(args: &Value, ctx: &SessionContext) -> Option<PersonId> {
    if let Some(r) = args["ref"].as_str().filter(|s| !s.is_empty()) {
        return Some(PersonId(r.to_string()));
    }
    ctx.messages.first().and_then(|m| m.person.clone())
}

pub(super) fn current_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.messages.first().and_then(|m| m.person.clone())
}
