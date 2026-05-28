use super::*;
use crate::identity::ClaimEvidence;
use crate::state::{ActorState, CoreTraits, DirectiveScope, GrowthConfig};
use crate::store::{
    ActionPromptSnapshotRecord, EventInboxRecord, IdentityDisclosureAudit, MemoryKind,
    MemorySource, MemoryType, MessageRole, PrivacyCategory, ReviewOutputAudit, TruthStatus,
    VisibilityScope,
};
use rusqlite::params;

mod action_log_tests;
mod conversation_tests;
mod debug_tests;
mod directive_tests;
mod event_inbox_tests;
mod group_tests;
mod helpers;
mod identity_tests;
mod intent_tests;
mod memory_lifecycle_tests;
mod memory_recall_tests;
mod schema_tests;
mod snapshot_tests;
mod social_graph_tests;
mod thought_tests;
use helpers::*;
