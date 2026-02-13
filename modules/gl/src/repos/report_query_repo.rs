//! Repository for report query operations (Phase 12)
//!
//! Provides read-only, bounded queries for reporting primitives.
//! All queries are tenant-scoped and designed to use indexes.
//!
//! **Performance Contract**: All queries must execute in < 500ms at normal scale (100K entries/tenant)

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

use crate::repos::account_repo::AccountType;

/// Errors that can occur during report query operations
#[derive(Debug, Error)]
pub enum ReportQueryError {
    #[error("Account not found: tenant_id={tenant_id}, code={code}")]
    AccountNotFound { tenant_id: String, code: String },

    #[error("Invalid date range: start {start} is after end {end}")]
    InvalidDateRange {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },

    #[error("Invalid pagination parameters: limit={limit}, offset={offset}")]
    InvalidPagination { limit: i64, offset: i64 },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================
// ACCOUNT ACTIVITY QUERIES
// ============================================================

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

// ============================================================
// GL DETAIL QUERIES
// ============================================================

/// GL detail entry header (for multi-account reporting)
#[derive(Debug, Clone, FromRow)]
pub struct GLDetailEntryHeader {
    pub id: Uuid,
    pub tenant_id: String,
    pub posted_at: DateTime<Utc>,
    pub description: Option<String>,
    pub currency: String,
    pub source_module: String,
}

/// GL detail line with account metadata
#[derive(Debug, Clone, FromRow)]
pub struct GLDetailLine {
    pub line_id: Uuid,
    pub journal_entry_id: Uuid,
    pub line_no: i32,
    pub account_code: String,
    pub account_name: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub memo: Option<String>,
}

/// Query journal entry IDs by date range only (no account filters)
///
/// Returns entry IDs ordered by posted_at DESC.
/// Uses index: `idx_journal_entries_tenant_posted`
pub async fn query_entries_by_date_range(
    pool: &PgPool,
    tenant_id: &str,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Uuid>, ReportQueryError> {
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

    let entry_ids = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT id
        FROM journal_entries
        WHERE tenant_id = $1
          AND posted_at >= $2
          AND posted_at <= $3
        ORDER BY posted_at DESC, created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(tenant_id)
    .bind(start_date)
    .bind(end_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(entry_ids)
}

/// Query journal entry IDs by date range and account codes
///
/// Returns entry IDs that have at least one line matching the specified account codes.
/// Complete entries (all lines) should be fetched separately using the returned IDs.
pub async fn query_entries_by_account_codes(
    pool: &PgPool,
    tenant_id: &str,
    account_codes: &[String],
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Uuid>, ReportQueryError> {
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

    let entry_ids = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT je.id
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        WHERE je.tenant_id = $1
          AND jl.account_ref = ANY($2)
          AND je.posted_at >= $3
          AND je.posted_at <= $4
        GROUP BY je.id, je.posted_at, je.created_at
        ORDER BY je.posted_at DESC, je.created_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(tenant_id)
    .bind(account_codes)
    .bind(start_date)
    .bind(end_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(entry_ids)
}

/// Query journal entry IDs by date range and account types
///
/// Returns entry IDs that have at least one line with an account of the specified type(s).
/// Complete entries (all lines) should be fetched separately using the returned IDs.
pub async fn query_entries_by_account_types(
    pool: &PgPool,
    tenant_id: &str,
    account_types: &[AccountType],
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Uuid>, ReportQueryError> {
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

    let entry_ids = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT je.id
        FROM journal_entries je
        INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
        INNER JOIN accounts a ON a.tenant_id = je.tenant_id AND a.code = jl.account_ref
        WHERE je.tenant_id = $1
          AND a.type = ANY($2)
          AND je.posted_at >= $3
          AND je.posted_at <= $4
        GROUP BY je.id, je.posted_at, je.created_at
        ORDER BY je.posted_at DESC, je.created_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(tenant_id)
    .bind(account_types)
    .bind(start_date)
    .bind(end_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(entry_ids)
}

/// Fetch a journal entry header by ID
pub async fn fetch_entry_header(
    pool: &PgPool,
    entry_id: Uuid,
) -> Result<Option<GLDetailEntryHeader>, ReportQueryError> {
    let header = sqlx::query_as::<_, GLDetailEntryHeader>(
        r#"
        SELECT id, tenant_id, posted_at, description, currency, source_module
        FROM journal_entries
        WHERE id = $1
        "#,
    )
    .bind(entry_id)
    .fetch_optional(pool)
    .await?;

    Ok(header)
}

/// Fetch all lines for a journal entry with account metadata
///
/// Returns lines joined with account metadata (code, name).
/// Ordered by line_no ASC.
pub async fn fetch_entry_lines_with_accounts(
    pool: &PgPool,
    entry_id: Uuid,
    tenant_id: &str,
) -> Result<Vec<GLDetailLine>, ReportQueryError> {
    let lines = sqlx::query_as::<_, GLDetailLine>(
        r#"
        SELECT
            jl.id as line_id,
            jl.journal_entry_id,
            jl.line_no,
            jl.account_ref as account_code,
            a.name as account_name,
            jl.debit_minor,
            jl.credit_minor,
            jl.memo
        FROM journal_lines jl
        INNER JOIN accounts a ON a.tenant_id = $2 AND a.code = jl.account_ref
        WHERE jl.journal_entry_id = $1
        ORDER BY jl.line_no ASC
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(lines)
}

/// Count journal entries matching date range (no account filters)
pub async fn count_entries_by_date_range(
    pool: &PgPool,
    tenant_id: &str,
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
        FROM journal_entries
        WHERE tenant_id = $1
          AND posted_at >= $2
          AND posted_at <= $3
        "#,
    )
    .bind(tenant_id)
    .bind(start_date)
    .bind(end_date)
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// Count journal entries matching date range and account codes
pub async fn count_entries_by_account_codes(
    pool: &PgPool,
    tenant_id: &str,
    account_codes: &[String],
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
        FROM (
            SELECT je.id
            FROM journal_entries je
            INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
            WHERE je.tenant_id = $1
              AND jl.account_ref = ANY($2)
              AND je.posted_at >= $3
              AND je.posted_at <= $4
            GROUP BY je.id
        ) AS distinct_entries
        "#,
    )
    .bind(tenant_id)
    .bind(account_codes)
    .bind(start_date)
    .bind(end_date)
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// Count journal entries matching date range and account types
pub async fn count_entries_by_account_types(
    pool: &PgPool,
    tenant_id: &str,
    account_types: &[AccountType],
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
        FROM (
            SELECT je.id
            FROM journal_entries je
            INNER JOIN journal_lines jl ON jl.journal_entry_id = je.id
            INNER JOIN accounts a ON a.tenant_id = je.tenant_id AND a.code = jl.account_ref
            WHERE je.tenant_id = $1
              AND a.type = ANY($2)
              AND je.posted_at >= $3
              AND je.posted_at <= $4
            GROUP BY je.id
        ) AS distinct_entries
        "#,
    )
    .bind(tenant_id)
    .bind(account_types)
    .bind(start_date)
    .bind(end_date)
    .fetch_one(pool)
    .await?;

    Ok(count)
}

// ============================================================
// PERIOD JOURNAL LISTING
// ============================================================

/// Period journal entry (header-only for listing)
#[derive(Debug, Clone, FromRow)]
pub struct PeriodJournalEntry {
    pub id: Uuid,
    pub posted_at: DateTime<Utc>,
    pub description: Option<String>,
    pub currency: String,
    pub source_module: String,
}

/// Query journal entries for a period (header-only listing)
///
/// Returns journal entry headers ordered by posted_at DESC.
/// Use this for period journal listings without fetching all lines.
pub async fn query_period_journal_entries(
    pool: &PgPool,
    tenant_id: &str,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<PeriodJournalEntry>, ReportQueryError> {
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

    let entries = sqlx::query_as::<_, PeriodJournalEntry>(
        r#"
        SELECT id, posted_at, description, currency, source_module
        FROM journal_entries
        WHERE tenant_id = $1
          AND posted_at >= $2
          AND posted_at <= $3
        ORDER BY posted_at DESC, created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(tenant_id)
    .bind(start_date)
    .bind(end_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_query_error_display() {
        let err = ReportQueryError::AccountNotFound {
            tenant_id: "tenant1".to_string(),
            code: "1000".to_string(),
        };
        assert!(err.to_string().contains("tenant1"));
        assert!(err.to_string().contains("1000"));

        let start = Utc::now();
        let end = start - chrono::Duration::hours(1);
        let err = ReportQueryError::InvalidDateRange { start, end };
        assert!(err.to_string().contains("is after"));
    }
}
