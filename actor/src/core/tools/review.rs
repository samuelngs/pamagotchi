use super::{SessionContext, SessionState};
use serde_json::Value;

pub fn tools() -> Vec<inference::Tool> {
    crate::core::review::tools()
}

pub async fn apply(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    crate::core::review::apply(args, ctx, state).await
}
