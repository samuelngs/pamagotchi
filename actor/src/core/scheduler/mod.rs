mod coalesce;
mod idle;
mod intents;

pub(crate) use coalesce::{drain_due_events, emit_due_consolidation};
pub(crate) use idle::take_due_scheduler_elapsed;
#[cfg(test)]
pub(crate) use intents::claim_and_send_due_intent;
pub(crate) use intents::drain_due_intents;

use crate::core::event::WakeEvent;
use crate::store::Store;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub(super) fn spawn_scheduler(
    event_tx: mpsc::Sender<WakeEvent>,
    store: Arc<dyn Store>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let scan_secs = 30u64;
        let idle_secs = 300u64;
        let consolidation_secs = 6 * 60 * 60u64;
        let interval = tokio::time::Duration::from_secs(scan_secs);
        let mut idle_elapsed = 0.0;
        let mut consolidation_elapsed = 0.0;
        let mut last_scan = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(interval).await;
            let elapsed_since_scan = last_scan.elapsed().as_secs_f64();
            last_scan = tokio::time::Instant::now();

            let now = chrono::Utc::now().timestamp();
            if !drain_due_intents(&event_tx, store.clone(), now, 32).await {
                break;
            }

            if !drain_due_events(&event_tx, store.clone(), now, 32).await {
                break;
            }

            if let Some(elapsed_secs) =
                take_due_scheduler_elapsed(&mut idle_elapsed, elapsed_since_scan, idle_secs as f64)
            {
                if event_tx
                    .send(WakeEvent::IdleTick { elapsed_secs })
                    .await
                    .is_err()
                {
                    break;
                }
            }

            if take_due_scheduler_elapsed(
                &mut consolidation_elapsed,
                elapsed_since_scan,
                consolidation_secs as f64,
            )
            .is_some()
                && !emit_due_consolidation(&event_tx, store.clone(), now).await
            {
                break;
            }
        }
    })
}
