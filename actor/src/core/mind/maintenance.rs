use super::*;

pub(super) const STALE_THOUGHT_SECS: i64 = 30 * 24 * 60 * 60;
const STALE_THOUGHT_MAX_IMPORTANCE: f32 = 0.3;
const STALE_THOUGHT_MAX_CONFIDENCE: f32 = 0.3;
const STALE_THOUGHT_PRUNE_LIMIT: usize = 1000;
pub(super) const STALE_MEMORY_SECS: i64 = 90 * 24 * 60 * 60;
const STALE_MEMORY_MAX_IMPORTANCE: f32 = 0.3;
const STALE_MEMORY_MAX_CONFIDENCE: f32 = 0.5;
const STALE_MEMORY_MAX_SENSITIVITY: f32 = 0.4;
const STALE_MEMORY_PRUNE_LIMIT: usize = 500;

impl Mind {
    pub(super) async fn prune_stale_thoughts(&self, now: i64) {
        let older_than = now.saturating_sub(STALE_THOUGHT_SECS);
        match self
            .store
            .prune_stale_thoughts(
                older_than,
                STALE_THOUGHT_MAX_IMPORTANCE,
                STALE_THOUGHT_MAX_CONFIDENCE,
                STALE_THOUGHT_PRUNE_LIMIT,
            )
            .await
        {
            Ok(count) if count > 0 => {
                self.metrics.record_thoughts_pruned(count);
                info!(
                    count,
                    "pruned stale low-signal thoughts during consolidation"
                );
            }
            Ok(_) => {}
            Err(e) => warn!(%e, "failed to prune stale thoughts during consolidation"),
        }
    }

    pub(super) async fn prune_stale_memories(&self, now: i64) {
        let older_than = now.saturating_sub(STALE_MEMORY_SECS);
        match self
            .store
            .prune_stale_memories(
                now,
                older_than,
                STALE_MEMORY_MAX_IMPORTANCE,
                STALE_MEMORY_MAX_CONFIDENCE,
                STALE_MEMORY_MAX_SENSITIVITY,
                STALE_MEMORY_PRUNE_LIMIT,
            )
            .await
        {
            Ok(count) if count > 0 => {
                self.metrics.record_memories_pruned(count);
                info!(
                    count,
                    "pruned stale low-signal memories during consolidation"
                );
            }
            Ok(_) => {}
            Err(e) => warn!(%e, "failed to prune stale memories during consolidation"),
        }
    }

    pub(super) async fn prune_stale_context(&self, now: i64) {
        self.prune_stale_thoughts(now).await;
        self.prune_stale_memories(now).await;
    }
}
