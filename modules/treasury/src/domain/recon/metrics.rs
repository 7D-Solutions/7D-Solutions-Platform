//! Reconciliation gauge snapshot — queries the database for current recon
//! state so the `/metrics` handler can set Prometheus gauges.
//!
//! No PII is included in any returned value.

use sqlx::PgPool;

use super::repo;

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
    repo::recon_snapshot(pool, app_id).await
}
