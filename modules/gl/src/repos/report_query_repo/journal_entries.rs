//! GL detail journal entry queries — multi-account reporting

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::repos::account_repo::AccountType;
use super::ReportQueryError;

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
