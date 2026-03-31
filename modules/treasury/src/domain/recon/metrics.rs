//! Reconciliation gauge snapshot — queries the database for current recon
//! state so the `/metrics` handler can set Prometheus gauges.
//!
//! No PII is included in any returned value.

use sqlx::PgPool;

/// Point-in-time reconciliation statistics.
pub struct ReconSnapshot {
    /// Active (non-superseded) matches.
    pub matched: i64,
    /// Imported statement lines still unmatched.
    pub unmatched_lines: i64,
    /// Payment-event transactions still unmatched.
    pub unmatched_txns: i64,
    /// matched / (matched + unmatched_lines), or 0 when no data.
    pub match_rate: f64,
}

/// Query the database for current recon gauge values scoped to a tenant.
pub async fn snapshot(pool: &PgPool, app_id: &str) -> Result<ReconSnapshot, sqlx::Error> {
    let matched: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_recon_matches WHERE superseded_by IS NULL AND app_id = $1",
    )
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    let unmatched_lines: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_transactions \
         WHERE status = 'unmatched' AND statement_id IS NOT NULL AND app_id = $1",
    )
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    let unmatched_txns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_transactions \
         WHERE status = 'unmatched' AND statement_id IS NULL AND app_id = $1",
    )
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    let total = matched + unmatched_lines;
    let match_rate = if total > 0 {
        matched as f64 / total as f64
    } else {
        0.0
    };

    Ok(ReconSnapshot {
        matched,
        unmatched_lines,
        unmatched_txns,
        match_rate,
    })
}
