use sqlx::PgPool;

/// Reconcile orphaned inflight rows older than threshold_secs.
/// Rejects threshold_secs <= 0 (would mark all inflight rows regardless of age).
pub async fn reconcile_orphan_inflight_pulls(
    pool: &PgPool,
    threshold_secs: i64,
) -> sqlx::Result<u64> {
    if threshold_secs <= 0 {
        return Ok(0);
    }
    let rows = sqlx::query(
        "UPDATE integrations_sync_pull_log
            SET status = 'failed',
                error = 'service_restart',
                completed_at = now()
          WHERE status = 'inflight'
            AND started_at < now() - make_interval(secs => $1::double precision)
         RETURNING id",
    )
    .bind(threshold_secs as f64)
    .fetch_all(pool)
    .await?;
    let count = rows.len() as u64;
    tracing::info!(count, threshold_secs, "reconciled orphan inflight pulls");
    Ok(count)
}
