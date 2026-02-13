//! GL Detail Service
//!
//! Provides read-only GL detail reporting: journal entries and lines for a tenant and period.
//! Supports optional filtering by account_code and currency.
//! Uses bounded queries with deterministic ordering to avoid full table scans.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::repos::period_repo;
use crate::repos::report_query_repo::{self, ReportQueryError};

/// GL detail response DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GLDetailResponse {
    pub tenant_id: String,
    pub period_start: String, // ISO 8601 timestamp
    pub period_end: String,   // ISO 8601 timestamp
    pub entries: Vec<GLDetailEntry>,
    pub pagination: PaginationMetadata,
}

/// GL detail entry (header + lines)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GLDetailEntry {
    pub id: String, // UUID as string
    pub posted_at: String, // ISO 8601 timestamp
    pub description: Option<String>,
    pub currency: String,
    pub source_module: String,
    pub lines: Vec<GLDetailEntryLine>,
}

/// GL detail line
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GLDetailEntryLine {
    pub line_no: i32,
    pub account_code: String,
    pub account_name: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub memo: Option<String>,
}

/// Pagination metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMetadata {
    pub limit: i64,
    pub offset: i64,
    pub total_count: i64,
    pub has_more: bool,
}

/// Errors that can occur during GL detail operations
#[derive(Debug, Error)]
pub enum GLDetailServiceError {
    #[error("Repository error: {0}")]
    Repo(#[from] ReportQueryError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Invalid period: {0}")]
    InvalidPeriod(String),

    #[error("Invalid currency: {0}")]
    InvalidCurrency(String),

    #[error("Invalid pagination: {0}")]
    InvalidPagination(String),

    #[error("Period not found: tenant_id={tenant_id}, period_id={period_id}")]
    PeriodNotFound { tenant_id: String, period_id: Uuid },
}

/// Get GL detail entries for a tenant and period
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `account_code` - Optional account code filter
/// * `currency` - Optional currency filter (ISO 4217)
/// * `limit` - Page size (1-100)
/// * `offset` - Pagination offset (>= 0)
///
/// # Returns
/// GL detail response with entries and pagination metadata
///
/// # Performance
/// Uses bounded queries with indexes to avoid table scans.
/// Expected: < 500ms at normal scale (100K entries/tenant)
pub async fn get_gl_detail(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    account_code: Option<&str>,
    currency: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<GLDetailResponse, GLDetailServiceError> {
    // Validate inputs
    validate_tenant_id(tenant_id)?;
    validate_pagination(limit, offset)?;

    if let Some(cur) = currency {
        validate_currency(cur)?;
    }

    // Fetch the accounting period to get date boundaries
    let period = period_repo::find_by_id(pool, period_id)
        .await
        .map_err(|e| match e {
            period_repo::PeriodError::Database(_) => GLDetailServiceError::Repo(
                ReportQueryError::Database(sqlx::Error::RowNotFound)
            ),
            _ => GLDetailServiceError::PeriodNotFound {
                tenant_id: tenant_id.to_string(),
                period_id,
            },
        })?
        .ok_or_else(|| GLDetailServiceError::PeriodNotFound {
            tenant_id: tenant_id.to_string(),
            period_id,
        })?;

    // Verify period belongs to tenant
    if period.tenant_id != tenant_id {
        return Err(GLDetailServiceError::InvalidPeriod(format!(
            "Period {} does not belong to tenant {}",
            period_id, tenant_id
        )));
    }

    // Convert NaiveDate to DateTime<Utc> (start of day and end of day)
    let period_start = period.period_start
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap())
        .and_utc();
    let period_end = period.period_end
        .and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap())
        .and_utc();

    // Query entry IDs based on filters
    let (entry_ids, total_count) = if let Some(account_code) = account_code {
        // Filter by account code
        let ids = report_query_repo::query_entries_by_account_codes(
            pool,
            tenant_id,
            &[account_code.to_string()],
            period_start,
            period_end,
            limit,
            offset,
        )
        .await?;

        let count = report_query_repo::count_entries_by_account_codes(
            pool,
            tenant_id,
            &[account_code.to_string()],
            period_start,
            period_end,
        )
        .await?;

        (ids, count)
    } else {
        // No account filter - query all entries in period
        let ids = report_query_repo::query_entries_by_date_range(
            pool,
            tenant_id,
            period_start,
            period_end,
            limit,
            offset,
        )
        .await?;

        let count = report_query_repo::count_entries_by_date_range(
            pool,
            tenant_id,
            period_start,
            period_end,
        )
        .await?;

        (ids, count)
    };

    // Fetch full entry details (header + lines) for each entry ID
    let mut entries = Vec::new();
    for entry_id in entry_ids {
        // Fetch header
        let header = report_query_repo::fetch_entry_header(pool, entry_id)
            .await?
            .ok_or_else(|| {
                ReportQueryError::Database(sqlx::Error::RowNotFound)
            })?;

        // Apply currency filter if specified
        if let Some(cur) = currency {
            if header.currency != cur {
                continue; // Skip entries not matching currency filter
            }
        }

        // Fetch lines with account metadata
        let lines = report_query_repo::fetch_entry_lines_with_accounts(
            pool,
            entry_id,
            tenant_id,
        )
        .await?;

        // Transform to DTO
        let entry = GLDetailEntry {
            id: header.id.to_string(),
            posted_at: header.posted_at.to_rfc3339(),
            description: header.description,
            currency: header.currency,
            source_module: header.source_module,
            lines: lines
                .into_iter()
                .map(|line| GLDetailEntryLine {
                    line_no: line.line_no,
                    account_code: line.account_code,
                    account_name: line.account_name,
                    debit_minor: line.debit_minor,
                    credit_minor: line.credit_minor,
                    memo: line.memo,
                })
                .collect(),
        };

        entries.push(entry);
    }

    // Build pagination metadata
    let has_more = offset + (entries.len() as i64) < total_count;

    Ok(GLDetailResponse {
        tenant_id: tenant_id.to_string(),
        period_start: period_start.to_rfc3339(),
        period_end: period_end.to_rfc3339(),
        entries,
        pagination: PaginationMetadata {
            limit,
            offset,
            total_count,
            has_more,
        },
    })
}

/// Validate tenant_id is not empty
fn validate_tenant_id(tenant_id: &str) -> Result<(), GLDetailServiceError> {
    if tenant_id.is_empty() {
        return Err(GLDetailServiceError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }
    Ok(())
}

/// Validate currency code (ISO 4217: 3 uppercase letters)
fn validate_currency(currency: &str) -> Result<(), GLDetailServiceError> {
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(GLDetailServiceError::InvalidCurrency(format!(
            "Currency must be 3 uppercase letters (ISO 4217): {}",
            currency
        )));
    }
    Ok(())
}

/// Validate pagination parameters
fn validate_pagination(limit: i64, offset: i64) -> Result<(), GLDetailServiceError> {
    if limit < 1 || limit > 100 {
        return Err(GLDetailServiceError::InvalidPagination(
            "limit must be between 1 and 100".to_string(),
        ));
    }
    if offset < 0 {
        return Err(GLDetailServiceError::InvalidPagination(
            "offset must be >= 0".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_tenant_id() {
        assert!(validate_tenant_id("tenant_123").is_ok());
        assert!(validate_tenant_id("").is_err());
    }

    #[test]
    fn test_validate_currency() {
        assert!(validate_currency("USD").is_ok());
        assert!(validate_currency("EUR").is_ok());
        assert!(validate_currency("usd").is_err()); // lowercase
        assert!(validate_currency("US").is_err()); // too short
        assert!(validate_currency("USDD").is_err()); // too long
    }

    #[test]
    fn test_validate_pagination() {
        assert!(validate_pagination(1, 0).is_ok());
        assert!(validate_pagination(50, 100).is_ok());
        assert!(validate_pagination(100, 0).is_ok());
        assert!(validate_pagination(0, 0).is_err()); // limit too low
        assert!(validate_pagination(101, 0).is_err()); // limit too high
        assert!(validate_pagination(50, -1).is_err()); // negative offset
    }
}
