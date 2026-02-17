//! Account activity queries — single-account transaction history

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::ReportQueryError;

/// Account activity line (single line from journal for an account)
///
/// Used for Account Activity Report: shows all transactions for a single account.
#[derive(Debug, Clone, FromRow)]
pub struct AccountActivityLine {
    pub entry_id: Uuid,
    pub posted_at: DateTime<Utc>,
    pub description: Option<String>,
    pub currency: String,
    pub line_id: Uuid,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub memo: Option<String>,
}

/// Query account activity for a single account over a date range
///
/// Returns journal lines for the specified account, ordered by posted_at ASC.
/// Uses index: `idx_journal_entries_tenant_posted` + `idx_journal_lines_entry_id`
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier (required for index usage)
/// * `account_code` - Chart of Accounts code (e.g., "1000")
/// * `start_date` - Start of date range (inclusive)
/// * `end_date` - End of date range (inclusive)
/// * `limit` - Max records to return (must be > 0)
/// * `offset` - Pagination offset (must be >= 0)
///
/// # Returns
/// Vector of account activity lines, ordered by posted_at ASC, line_no ASC
///
/// # Performance
/// Expected: < 200ms for 1000 transactions (per Phase 12 spec)
pub async fn query_account_activity(
    pool: &PgPool,
    tenant_id: &str,
    account_code: &str,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AccountActivityLine>, ReportQueryError> {
    // Validate date range
    if start_date > end_date {
        return Err(ReportQueryError::InvalidDateRange {
            start: start_date,
            end: end_date,
        });
    }

    // Validate pagination
    if limit <= 0 || offset < 0 {
        return Err(ReportQueryError::InvalidPagination { limit, offset });
    }

    let lines = sqlx::query_as::<_, AccountActivityLine>(
        r#"
        SELECT
            je.id as entry_id,
            je.posted_at,
            je.description,
            je.currency,
            jl.id as line_id,
            jl.debit_minor,
            jl.credit_minor,
            jl.memo
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        WHERE je.tenant_id = $1
          AND jl.account_ref = $2
          AND je.posted_at >= $3
          AND je.posted_at <= $4
        ORDER BY je.posted_at ASC, jl.line_no ASC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(tenant_id)
    .bind(account_code)
    .bind(start_date)
    .bind(end_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(lines)
}

/// Count total account activity lines for pagination metadata
///
/// Returns the total count of lines matching the query (without limit/offset).
pub async fn count_account_activity(
    pool: &PgPool,
    tenant_id: &str,
    account_code: &str,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
) -> Result<i64, ReportQueryError> {
    // Validate date range
    if start_date > end_date {
        return Err(ReportQueryError::InvalidDateRange {
            start: start_date,
            end: end_date,
        });
    }

    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        WHERE je.tenant_id = $1
          AND jl.account_ref = $2
          AND je.posted_at >= $3
          AND je.posted_at <= $4
        "#,
    )
    .bind(tenant_id)
    .bind(account_code)
    .bind(start_date)
    .bind(end_date)
    .fetch_one(pool)
    .await?;

    Ok(count)
}
