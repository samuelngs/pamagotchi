use super::context::{
    SessionContext, SessionKind, SessionState, TYPING_ACTIVE_SECS, TypingStateKey,
};
use crate::core::ActionKind;
use crate::state::{RelationshipChange, RelationshipInteraction, RelationshipStanding};
use crate::store::{
    ActionMessageRecord, IntentRecord, MessageRole, OutboundDeliveryRecord, StoredMessage,
};
use inference::Tool;
use protocol::{
    ConversationId, GroupId, InboundMessage, MediaAssetId, MediaAttachment, MediaKind, PersonId,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tracing::warn;

const TYPING_SEND_WAIT_MAX_MS: u64 = 1_500;
const TYPING_SEND_POLL_MS: u64 = 100;

mod attachments;
mod delivery;
mod read;
mod schemas;
mod send;
mod summary;
mod target;
mod typing;

#[cfg(test)]
pub use read::read;
pub use read::read_with_state;
pub use schemas::tools;
pub use send::send;
pub use summary::update_conversation_summary;

#[cfg(test)]
mod tests;
