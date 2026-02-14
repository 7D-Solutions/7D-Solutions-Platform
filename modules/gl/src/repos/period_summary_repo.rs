//! Period Summary Repository
//!
//! Provides data access for period summaries, preferring precomputed snapshots
//! when available, otherwise computing from account_balances.

use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

/// Period summary model (either from snapshot or computed from balances)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PeriodSummary {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String,
    pub journal_count: i32,
    pub line_count: i32,
    pub total_debits_minor: i64,
    pub total_credits_minor: i64,
    pub is_snapshot: bool, // true if from snapshot, false if computed
    pub snapshot_created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Errors that can occur during period summary operations
#[derive(Debug, Error)]
pub enum PeriodSummaryError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Period not found: {0}")]
    PeriodNotFound(Uuid),
}

/// Find period summary for a tenant and period, optionally filtered by currency
///
/// Prefers precomputed snapshot if present, otherwise computes from account_balances.
/// Does NOT scan journal_lines in normal operation.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Optional currency filter (None = all currencies, sum across all)
///
/// # Returns
/// Period summary with counts and totals
pub async fn find_period_summary(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<PeriodSummary, PeriodSummaryError> {
    // First, verify the period exists
    let period_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM accounting_periods WHERE id = $1)",
    )
    .bind(period_id)
    .fetch_one(pool)
    .await?;

    if !period_exists {
        return Err(PeriodSummaryError::PeriodNotFound(period_id));
    }

    // Try to find precomputed snapshot first
    if let Some(cur) = currency {
        if let Some(snapshot) = find_snapshot_by_currency(pool, tenant_id, period_id, cur).await? {
            return Ok(snapshot);
        }
    } else {
        // No currency specified - try to find aggregated snapshot or compute from balances
        if let Some(snapshot) = find_aggregated_snapshot(pool, tenant_id, period_id).await? {
            return Ok(snapshot);
        }
    }

    // No snapshot found - compute from account_balances (NOT journal_lines)
    compute_from_balances(pool, tenant_id, period_id, currency).await
}

/// Find snapshot for a specific currency
async fn find_snapshot_by_currency(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,
) -> Result<Option<PeriodSummary>, PeriodSummaryError> {
    let snapshot = sqlx::query_as::<_, PeriodSummarySnapshot>(
        r#"
        SELECT
            tenant_id,
            period_id,
            currency,
            journal_count,
            line_count,
            total_debits_minor,
            total_credits_minor,
            created_at
        FROM period_summary_snapshots
        WHERE tenant_id = $1
          AND period_id = $2
          AND currency = $3
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(currency)
    .fetch_optional(pool)
    .await?;

    Ok(snapshot.map(|s| PeriodSummary {
        tenant_id: s.tenant_id,
        period_id: s.period_id,
        currency: s.currency,
        journal_count: s.journal_count,
        line_count: s.line_count,
        total_debits_minor: s.total_debits_minor,
        total_credits_minor: s.total_credits_minor,
        is_snapshot: true,
        snapshot_created_at: Some(s.created_at),
    }))
}

/// Find aggregated snapshot across all currencies
async fn find_aggregated_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Option<PeriodSummary>, PeriodSummaryError> {
    // Aggregate all snapshots for this tenant + period across currencies
    let snapshot = sqlx::query_as::<_, AggregatedSnapshot>(
        r#"
        SELECT
            tenant_id,
            period_id,
            SUM(journal_count)::INTEGER as journal_count,
            SUM(line_count)::INTEGER as line_count,
            SUM(total_debits_minor)::BIGINT as total_debits_minor,
            SUM(total_credits_minor)::BIGINT as total_credits_minor,
            MAX(created_at) as latest_created_at
        FROM period_summary_snapshots
        WHERE tenant_id = $1
          AND period_id = $2
        GROUP BY tenant_id, period_id
        HAVING COUNT(*) > 0
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_optional(pool)
    .await?;

    Ok(snapshot.map(|s| PeriodSummary {
        tenant_id: s.tenant_id,
        period_id: s.period_id,
        currency: "MULTI".to_string(), // Indicate multi-currency aggregate
        journal_count: s.journal_count,
        line_count: s.line_count,
        total_debits_minor: s.total_debits_minor,
        total_credits_minor: s.total_credits_minor,
        is_snapshot: true,
        snapshot_created_at: Some(s.latest_created_at),
    }))
}

/// Compute summary from account_balances (fallback when no snapshot exists)
///
/// This does NOT scan journal_lines - it aggregates account_balances instead.
/// Note: journal_count and line_count will be 0 when computed from balances,
/// as these metrics cannot be accurately determined without scanning journal_lines.
async fn compute_from_balances(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<PeriodSummary, PeriodSummaryError> {
    // Aggregate account balances for this period
    // Note: We can only compute totals, not counts, from account_balances
    let computed = if let Some(cur) = currency {
        sqlx::query_as::<_, ComputedSummary>(
            r#"
            SELECT
                $1::TEXT as tenant_id,
                $2::UUID as period_id,
                $3::TEXT as currency,
                0::INTEGER as journal_count, -- Cannot determine from balances
                0::INTEGER as line_count, -- Cannot determine from balances
                COALESCE(SUM(ab.debit_total_minor), 0)::BIGINT as total_debits_minor,
                COALESCE(SUM(ab.credit_total_minor), 0)::BIGINT as total_credits_minor
            FROM account_balances ab
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND ab.currency = $3
            "#,
        )
        .bind(tenant_id)
        .bind(period_id)
        .bind(cur)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as::<_, ComputedSummary>(
            r#"
            SELECT
                $1::TEXT as tenant_id,
                $2::UUID as period_id,
                'MULTI'::TEXT as currency,
                0::INTEGER as journal_count, -- Cannot determine from balances
                0::INTEGER as line_count, -- Cannot determine from balances
                COALESCE(SUM(ab.debit_total_minor), 0)::BIGINT as total_debits_minor,
                COALESCE(SUM(ab.credit_total_minor), 0)::BIGINT as total_credits_minor
            FROM account_balances ab
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
            "#,
        )
        .bind(tenant_id)
        .bind(period_id)
        .fetch_one(pool)
        .await?
    };

    Ok(PeriodSummary {
        tenant_id: computed.tenant_id,
        period_id: computed.period_id,
        currency: computed.currency,
        journal_count: computed.journal_count,
        line_count: computed.line_count,
        total_debits_minor: computed.total_debits_minor,
        total_credits_minor: computed.total_credits_minor,
        is_snapshot: false,
        snapshot_created_at: None,
    })
}

/// Database model for period_summary_snapshots row
#[derive(Debug, Clone, sqlx::FromRow)]
struct PeriodSummarySnapshot {
    tenant_id: String,
    period_id: Uuid,
    currency: String,
    journal_count: i32,
    line_count: i32,
    total_debits_minor: i64,
    total_credits_minor: i64,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Aggregated snapshot across multiple currencies
#[derive(Debug, Clone, sqlx::FromRow)]
struct AggregatedSnapshot {
    tenant_id: String,
    period_id: Uuid,
    journal_count: i32,
    line_count: i32,
    total_debits_minor: i64,
    total_credits_minor: i64,
    latest_created_at: chrono::DateTime<chrono::Utc>,
}

/// Computed summary from account_balances
#[derive(Debug, Clone, sqlx::FromRow)]
struct ComputedSummary {
    tenant_id: String,
    period_id: Uuid,
    currency: String,
    journal_count: i32,
    line_count: i32,
    total_debits_minor: i64,
    total_credits_minor: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_summary_model() {
        let summary = PeriodSummary {
            tenant_id: "tenant_123".to_string(),
            period_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            journal_count: 10,
            line_count: 20,
            total_debits_minor: 100000,
            total_credits_minor: 100000,
            is_snapshot: true,
            snapshot_created_at: Some(chrono::Utc::now()),
        };

        assert_eq!(summary.tenant_id, "tenant_123");
        assert_eq!(summary.journal_count, 10);
        assert!(summary.is_snapshot);
    }
}
