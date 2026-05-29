mod action;
pub(crate) mod context;
mod conversation;
mod directives;
mod format;
mod identity;
mod mind;
mod person;
mod profile;
mod templates;

use action::build_action;
use mind::build_mind;
#[cfg(test)]
use templates::action_task_template;
use templates::make_env;

use super::action::ActionKind;
use super::handle::StateHandle;
use super::review;
use super::social_read;
use super::tools::{SessionContext, SessionKind};
use crate::state::{ActorState, RelationshipStanding};
use crate::store::{ConversationSummary, MemorySubjectType, MemoryType, Store};
use context::*;
use minijinja::Environment;
use protocol::{ConversationId, GroupId, InboundMessage, ProfileId};
use std::sync::Arc;

pub async fn build_system_prompt(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    kind: &SessionKind,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    session_ctx: &SessionContext,
    relationship_standing: &RelationshipStanding,
) -> anyhow::Result<String> {
    let env = make_env();
    match kind {
        SessionKind::Mind => {
            build_mind(
                &env,
                state,
                store,
                messages,
                session_ctx,
                &session_ctx.concurrent_summaries,
            )
            .await
        }
        SessionKind::Action(action_kind) => {
            build_action(
                &env,
                state,
                store,
                action_kind,
                messages,
                conversation,
                session_ctx,
                relationship_standing,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests;
