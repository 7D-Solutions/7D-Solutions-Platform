use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;

use super::repo::{claim_due_batch, mark_sent, reschedule_or_fail, reset_orphaned_claims};
use super::sender::NotificationSender;

/// Counts returned by a single `dispatch_once` cycle.
#[derive(Debug, Default)]
pub struct DispatchResult {
    /// Orphaned `claimed` rows reset to `pending` at startup/cycle start.
    pub reset_count: u64,
    /// Rows atomically claimed in this cycle.
    pub claimed_count: usize,
    /// Rows successfully delivered and marked `sent`.
    pub sent_count: usize,
    /// Rows that failed delivery and were rescheduled for retry.
    pub rescheduled_count: usize,
    /// Rows that exceeded the retry limit and were marked `failed`.
    pub failed_count: usize,
}

/// Run one full dispatch cycle:
///
/// 1. Reset orphaned `claimed` rows older than 5 minutes back to `pending`.
/// 2. Claim up to 100 due notifications (`FOR UPDATE SKIP LOCKED`).
/// 3. For each claimed row, attempt delivery:
///    - success → `mark_sent`
///    - failure → `reschedule_or_fail` (linear back-off, max 5 retries)
/// 4. Return a `DispatchResult` with per-category counts.
pub async fn dispatch_once(
    pool: &PgPool,
    sender: Arc<dyn NotificationSender>,
) -> anyhow::Result<DispatchResult> {
    let mut result = DispatchResult::default();

    // 1. Reset orphaned claims (claimed but stale — likely from a prior crash).
    let cutoff = Utc::now() - chrono::Duration::minutes(5);
    result.reset_count = reset_orphaned_claims(pool, cutoff)
        .await
        .map_err(|e| anyhow::anyhow!("reset_orphaned_claims failed: {e}"))?;

    if result.reset_count > 0 {
        tracing::warn!(reset_count = result.reset_count, "reset orphaned claimed notifications");
    }

    // 2. Claim due batch (FOR UPDATE SKIP LOCKED enforced inside repo).
    let batch = claim_due_batch(pool, 100)
        .await
        .map_err(|e| anyhow::anyhow!("claim_due_batch failed: {e}"))?;

    result.claimed_count = batch.len();

    // 3. Attempt delivery for each claimed notification.
    for notif in &batch {
        match sender.send(notif).await {
            Ok(()) => {
                mark_sent(pool, notif.id)
                    .await
                    .map_err(|e| anyhow::anyhow!("mark_sent failed for {}: {e}", notif.id))?;
                result.sent_count += 1;
            }
            Err(err) => {
                tracing::warn!(
                    id = %notif.id,
                    retry_count = notif.retry_count,
                    error = %err,
                    "notification delivery failed"
                );
                reschedule_or_fail(pool, notif.id, notif.retry_count)
                    .await
                    .map_err(|e| anyhow::anyhow!("reschedule_or_fail failed for {}: {e}", notif.id))?;

                if notif.retry_count >= 5 {
                    result.failed_count += 1;
                } else {
                    result.rescheduled_count += 1;
                }
            }
        }
    }

    tracing::debug!(
        reset = result.reset_count,
        claimed = result.claimed_count,
        sent = result.sent_count,
        rescheduled = result.rescheduled_count,
        failed = result.failed_count,
        "dispatch_once complete"
    );

    Ok(result)
}
