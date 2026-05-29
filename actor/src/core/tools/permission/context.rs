use crate::core::ActionKind;
use crate::core::tools::{SessionContext, SessionKind};
use crate::state::RelationshipStanding;
use protocol::ConversationId;

pub(super) fn privileged_profile_write(ctx: &SessionContext) -> bool {
    matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
        || matches!(
            ctx.kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        )
}

pub(super) fn privileged_sensitive_recall(ctx: &SessionContext) -> bool {
    matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
        || matches!(
            ctx.kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        )
}

pub(super) fn privileged_memory_recall(ctx: &SessionContext) -> bool {
    matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
        || matches!(
            ctx.kind,
            SessionKind::Action(
                ActionKind::Review | ActionKind::Consolidate | ActionKind::Ruminate
            )
        )
}

pub(super) fn privileged_conversation_read(ctx: &SessionContext) -> bool {
    matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
        || matches!(
            ctx.kind,
            SessionKind::Action(
                ActionKind::Review | ActionKind::Consolidate | ActionKind::Ruminate
            )
        )
}

pub(super) fn privileged_intent_write(ctx: &SessionContext) -> bool {
    matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
        || matches!(
            ctx.kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        )
}

pub(super) fn privileged_intent_create(ctx: &SessionContext) -> bool {
    privileged_intent_write(ctx) || matches!(ctx.kind, SessionKind::Action(ActionKind::Ruminate))
}

pub(super) fn current_person(ctx: &SessionContext) -> Option<&str> {
    ctx.messages
        .first()
        .and_then(|message| message.person.as_ref())
        .map(|id| id.0.as_str())
}

pub(super) fn current_identity(ctx: &SessionContext) -> Option<&str> {
    ctx.messages
        .first()
        .and_then(|message| message.identity.as_ref())
        .map(|id| id.0.as_str())
}

pub(super) fn current_profile(ctx: &SessionContext) -> Option<&str> {
    ctx.messages
        .first()
        .and_then(|message| message.profile.as_ref())
        .map(|id| id.0.as_str())
}

pub(super) fn current_conversation(ctx: &SessionContext) -> Option<&str> {
    ctx.conversation
        .as_ref()
        .or_else(|| ctx.messages.first().map(|message| &message.conversation))
        .map(|id| id.0.as_str())
}

pub(super) fn current_conversation_id(ctx: &SessionContext) -> Option<ConversationId> {
    ctx.conversation.clone().or_else(|| {
        ctx.messages
            .first()
            .map(|message| message.conversation.clone())
    })
}
