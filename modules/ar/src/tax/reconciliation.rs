//! Platform vs QBO tax divergence detection worker (bd-jsbai).
//!
//! Reconciles platform-computed tax against QBO's Automated Sales Tax
//! for tenants in dual-source mode, writing divergence rows and raising
//! flags for outliers.
//!
//! ## Invariants
//!
//! Rows in `ar_tax_reconciliation_log` are immutable once inserted.
//! Only review fields (`reviewed_by`, `reviewed_at`, `resolution`) may be
//! updated after insertion — use `mark_reviewed`.
//!
//! When `qbo_tax_cents = 0` and `platform_tax_cents != 0`, the divergence_pct
//! column is NULL (cannot express as a ratio). Any non-zero divergence when one
//! side is zero is flagged unconditionally.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Model
// ============================================================================

/// A single reconciliation log row as returned by the admin API.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaxReconciliationRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub invoice_id: Uuid,
    pub platform_tax_cents: i64,
    pub qbo_tax_cents: i64,
    pub divergence_cents: i64,
    pub divergence_pct: Option<f64>,
    pub flagged: bool,
    pub detected_at: DateTime<Utc>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
}

/// Parameters for listing reconciliation rows.
#[derive(Debug, Default)]
pub struct ListReconciliationFilter {
    pub tenant_id: Option<Uuid>,
    pub flagged_only: bool,
    pub resolution: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

// ============================================================================
// Core logic
// ============================================================================

/// Determine whether a divergence should be flagged.
///
/// Flagging rules:
/// - If either side is zero and the other is not: always flag.
/// - Otherwise: flag if |divergence_pct| > threshold.
pub fn should_flag(
    platform_tax_cents: i64,
    qbo_tax_cents: i64,
    threshold: f64,
) -> bool {
    if qbo_tax_cents == 0 {
        return platform_tax_cents != 0;
    }
    let pct = (platform_tax_cents - qbo_tax_cents).abs() as f64 / qbo_tax_cents.abs() as f64;
    pct > threshold
}

// ============================================================================
// Repository
// ============================================================================

/// Fetch the per-tenant reconciliation threshold; falls back to 0.005 (0.5%).
pub async fn get_threshold(pool: &PgPool, tenant_id: Uuid) -> Result<f64, sqlx::Error> {
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT reconciliation_threshold_pct::float8 \
         FROM ar_tenant_tax_config WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(t,)| t).unwrap_or(0.005))
}

/// Insert a new reconciliation log row.
///
/// The `flagged` field is computed by the caller via `should_flag`.
pub async fn insert_reconciliation_row(
    pool: &PgPool,
    tenant_id: Uuid,
    invoice_id: Uuid,
    platform_tax_cents: i64,
    qbo_tax_cents: i64,
    flagged: bool,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ar_tax_reconciliation_log \
         (id, tenant_id, invoice_id, platform_tax_cents, qbo_tax_cents, flagged) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(invoice_id)
    .bind(platform_tax_cents)
    .bind(qbo_tax_cents)
    .bind(flagged)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Fetch flagged rows for the admin API.
pub async fn list_reconciliation_rows(
    pool: &PgPool,
    filter: &ListReconciliationFilter,
    page: i64,
    page_size: i64,
) -> Result<Vec<TaxReconciliationRow>, sqlx::Error> {
    let mut q = sqlx::QueryBuilder::new(
        "SELECT id, tenant_id, invoice_id, platform_tax_cents, qbo_tax_cents, \
         divergence_cents, divergence_pct::float8 AS divergence_pct, flagged, detected_at, \
         reviewed_by, reviewed_at, resolution \
         FROM ar_tax_reconciliation_log WHERE 1=1",
    );
    if let Some(tid) = filter.tenant_id {
        q.push(" AND tenant_id = ");
        q.push_bind(tid);
    }
    if filter.flagged_only {
        q.push(" AND flagged = true");
    }
    if let Some(ref res) = filter.resolution {
        q.push(" AND resolution = ");
        q.push_bind(res.clone());
    }
    if let Some(from) = filter.from {
        q.push(" AND detected_at >= ");
        q.push_bind(from);
    }
    if let Some(to) = filter.to {
        q.push(" AND detected_at <= ");
        q.push_bind(to);
    }
    q.push(" ORDER BY detected_at DESC");
    q.push(" LIMIT ");
    q.push_bind(page_size);
    q.push(" OFFSET ");
    q.push_bind((page - 1) * page_size);

    q.build_query_as().fetch_all(pool).await
}

/// Count matching rows (for pagination metadata).
pub async fn count_reconciliation_rows(
    pool: &PgPool,
    filter: &ListReconciliationFilter,
) -> Result<i64, sqlx::Error> {
    let mut q = sqlx::QueryBuilder::new(
        "SELECT COUNT(*) FROM ar_tax_reconciliation_log WHERE 1=1",
    );
    if let Some(tid) = filter.tenant_id {
        q.push(" AND tenant_id = ");
        q.push_bind(tid);
    }
    if filter.flagged_only {
        q.push(" AND flagged = true");
    }
    if let Some(ref res) = filter.resolution {
        q.push(" AND resolution = ");
        q.push_bind(res.clone());
    }
    if let Some(from) = filter.from {
        q.push(" AND detected_at >= ");
        q.push_bind(from);
    }
    if let Some(to) = filter.to {
        q.push(" AND detected_at <= ");
        q.push_bind(to);
    }
    let (count,): (i64,) = q.build_query_as().fetch_one(pool).await?;
    Ok(count)
}

/// Record a review decision on an existing row.
pub async fn mark_reviewed(
    pool: &PgPool,
    row_id: Uuid,
    reviewed_by: Uuid,
    resolution: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ar_tax_reconciliation_log \
         SET reviewed_by = $1, reviewed_at = NOW(), resolution = $2 \
         WHERE id = $3",
    )
    .bind(reviewed_by)
    .bind(resolution)
    .bind(row_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── should_flag unit tests ────────────────────────────────────────────────

    #[test]
    fn tax_reconciliation_identical_values_no_flag() {
        // platform=$5.00, qbo=$5.00 → 0% divergence, not flagged
        assert!(!should_flag(500, 500, 0.005));
    }

    #[test]
    fn tax_reconciliation_small_divergence_within_threshold_no_flag() {
        // platform=$5.00, qbo=$5.01 → 0.2% divergence, threshold 0.5% → not flagged
        assert!(!should_flag(500, 501, 0.005));
    }

    #[test]
    fn tax_reconciliation_large_divergence_exceeds_threshold_flags() {
        // platform=$5.00, qbo=$5.50 → 9.1% divergence → flagged
        assert!(should_flag(500, 550, 0.005));
    }

    #[test]
    fn tax_reconciliation_zero_qbo_tax_platform_nonzero_flags() {
        // qbo=$0, platform=$5 → divergence_pct NULL; flag unconditionally
        assert!(should_flag(500, 0, 0.005));
    }

    #[test]
    fn tax_reconciliation_flag_threshold_exactly_at_boundary_not_flagged() {
        // Exactly at threshold (pct == threshold) → not flagged (strict >)
        // platform = 1005, qbo = 1000 → pct = 0.005 exactly
        assert!(!should_flag(1005, 1000, 0.005));
    }

    #[test]
    fn tax_reconciliation_flag_threshold_one_cent_above_boundary_flagged() {
        // platform = 1006, qbo = 1000 → pct = 0.006 > 0.005 → flagged
        assert!(should_flag(1006, 1000, 0.005));
    }

    #[test]
    fn tax_reconciliation_zero_both_sides_no_flag() {
        // Both zero → trivially identical, not flagged
        assert!(!should_flag(0, 0, 0.005));
    }
}
