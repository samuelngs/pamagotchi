use super::rows::read_memory;
use super::support::{
    SlowSqliteQuery, TxGuard, bytes_to_embedding, embedding_to_bytes,
    write_memory_embedding_best_effort,
};
use crate::store::{
    Memory, MemoryMutationRecord, MemorySubject, MemorySubjectType, MemoryUpdate, PrivacyCategory,
    RecallQuery, TruthStatus,
};
use protocol::MemoryId;
use rusqlite::{Connection, OptionalExtension, params};

mod recall;
mod seed;

mod forget;
mod mutations;
mod prune;
mod read;
mod update;
mod write;

pub(super) use forget::forget;
pub(super) use mutations::memory_mutations_for_memory;
pub(super) use prune::prune_stale_memories;
pub(super) use read::{get_memory, memories_for_subject};
pub(super) use update::update_memory;
pub(super) use write::store_memory;

pub(super) fn seed_actor_identity_memories(conn: &Connection) -> anyhow::Result<()> {
    seed::seed_actor_identity_memories(conn)
}

pub(super) fn recall(conn: &Connection, query: &RecallQuery) -> anyhow::Result<Vec<Memory>> {
    recall::recall(conn, query)
}
