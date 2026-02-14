//! Account Activity Service
//!
//! Provides read-only account activity reporting: journal lines for a specific account
//! within a period or date range. Uses bounded queries with deterministic ordering.

use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::repos::report_query_repo::{self, ReportQueryError};
use crate::repos::period_repo;

/// Account activity response DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountActivityResponse {
    pub tenant_id: String,
    pub account_code: String,
    pub period_start: String, // ISO 8601 timestamp
    pub period_end: String,   // ISO 8601 timestamp
    pub lines: Vec<AccountActivityLine>,
    pub pagination: PaginationMetadata,
}

/// Account activity line (single transaction line)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountActivityLine {
    pub entry_id: String, // UUID as string
    pub posted_at: String, // ISO 8601 timestamp
    pub description: Option<String>,
    pub currency: String,
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

/// Errors that can occur during account activity operations
#[derive(Debug, Error)]
pub enum AccountActivityServiceError {
    #[error("Repository error: {0}")]
    Repo(#[from] ReportQueryError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Invalid account_code: {0}")]
    InvalidAccountCode(String),

    #[error("Invalid period: {0}")]
    InvalidPeriod(String),

    #[error("Invalid currency: {0}")]
    InvalidCurrency(String),

    #[error("Invalid pagination: {0}")]
    InvalidPagination(String),

    #[error("Invalid date range: {0}")]
    InvalidDateRange(String),

    #[error("Period not found: tenant_id={tenant_id}, period_id={period_id}")]
    PeriodNotFound { tenant_id: String, period_id: Uuid },

    #[error("Period or date range required")]
    MissingDateFilter,
}

/// Get account activity for a specific account
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `account_code` - Chart of Accounts code (e.g., "1000")
/// * `period_id` - Optional accounting period UUID
/// * `start_date` - Optional start date (required if no period_id)
/// * `end_date` - Optional end date (required if no period_id)
/// * `currency` - Optional currency filter (ISO 4217)
/// * `limit` - Page size (1-100)
/// * `offset` - Pagination offset (>= 0)
///
/// # Returns
/// Account activity response with lines and pagination metadata
///
/// # Performance
/// Uses bounded queries with indexes to avoid table scans.
/// Expected: < 200ms for 1000 transactions (per Phase 12 spec)
pub async fn get_account_activity(
    pool: &PgPool,
    tenant_id: &str,
    account_code: &str,
    period_id: Option<Uuid>,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
    currency: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<AccountActivityResponse, AccountActivityServiceError> {
    // Validate inputs
    validate_tenant_id(tenant_id)?;
    validate_account_code(account_code)?;
    validate_pagination(limit, offset)?;

    if let Some(cur) = currency {
        validate_currency(cur)?;
    }

    // Determine date range: either from period_id or from start_date/end_date
    let (period_start, period_end) = if let Some(period_id) = period_id {
        // Fetch the accounting period to get date boundaries
        let period = period_repo::find_by_id(pool, period_id)
            .await
            .map_err(|e| match e {
                period_repo::PeriodError::Database(_) => AccountActivityServiceError::Repo(
                    ReportQueryError::Database(sqlx::Error::RowNotFound)
                ),
                _ => AccountActivityServiceError::PeriodNotFound {
                    tenant_id: tenant_id.to_string(),
                    period_id,
                },
            })?
            .ok_or_else(|| AccountActivityServiceError::PeriodNotFound {
                tenant_id: tenant_id.to_string(),
                period_id,
            })?;

        // Verify period belongs to tenant
        if period.tenant_id != tenant_id {
            return Err(AccountActivityServiceError::InvalidPeriod(format!(
                "Period {} does not belong to tenant {}",
                period_id, tenant_id
            )));
        }

        // Convert NaiveDate to DateTime<Utc> (start of day and end of day)
        let start = period.period_start
            .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap())
            .and_utc();
        let end = period.period_end
            .and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap())
            .and_utc();

        (start, end)
    } else if let (Some(start), Some(end)) = (start_date, end_date) {
        // Use provided date range
        if start > end {
            return Err(AccountActivityServiceError::InvalidDateRange(
                format!("start_date {} is after end_date {}", start, end)
            ));
        }
        (start, end)
    } else {
        // Neither period_id nor date range provided
        return Err(AccountActivityServiceError::MissingDateFilter);
    };

    // Query account activity lines
    let lines = report_query_repo::query_account_activity(
        pool,
        tenant_id,
        account_code,
        period_start,
        period_end,
        limit,
        offset,
    )
    .await?;

    // Get total count for pagination
    let total_count = report_query_repo::count_account_activity(
        pool,
        tenant_id,
        account_code,
        period_start,
        period_end,
    )
    .await?;

    // Apply currency filter if specified (post-query filter)
    let filtered_lines: Vec<_> = if let Some(cur) = currency {
        lines
            .into_iter()
            .filter(|line| line.currency == cur)
            .collect()
    } else {
        lines
    };

    // Transform to DTO
    let response_lines = filtered_lines
        .into_iter()
        .map(|line| AccountActivityLine {
            entry_id: line.entry_id.to_string(),
            posted_at: line.posted_at.to_rfc3339(),
            description: line.description,
            currency: line.currency,
            debit_minor: line.debit_minor,
            credit_minor: line.credit_minor,
            memo: line.memo,
        })
        .collect::<Vec<_>>();

    // Build pagination metadata
    let has_more = offset + (response_lines.len() as i64) < total_count;

    Ok(AccountActivityResponse {
        tenant_id: tenant_id.to_string(),
        account_code: account_code.to_string(),
        period_start: period_start.to_rfc3339(),
        period_end: period_end.to_rfc3339(),
        lines: response_lines,
        pagination: PaginationMetadata {
            limit,
            offset,
            total_count,
            has_more,
        },
    })
}

/// Validate tenant_id is not empty
fn validate_tenant_id(tenant_id: &str) -> Result<(), AccountActivityServiceError> {
    if tenant_id.is_empty() {
        return Err(AccountActivityServiceError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }
    Ok(())
}

/// Validate account_code is not empty
fn validate_account_code(account_code: &str) -> Result<(), AccountActivityServiceError> {
    if account_code.is_empty() {
        return Err(AccountActivityServiceError::InvalidAccountCode(
            "account_code cannot be empty".to_string(),
        ));
    }
    Ok(())
}

/// Validate currency code (ISO 4217: 3 uppercase letters)
fn validate_currency(currency: &str) -> Result<(), AccountActivityServiceError> {
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(AccountActivityServiceError::InvalidCurrency(format!(
            "Currency must be 3 uppercase letters (ISO 4217): {}",
            currency
        )));
    }
    Ok(())
}

/// Validate pagination parameters
fn validate_pagination(limit: i64, offset: i64) -> Result<(), AccountActivityServiceError> {
    if limit < 1 || limit > 100 {
        return Err(AccountActivityServiceError::InvalidPagination(
            "limit must be between 1 and 100".to_string(),
        ));
    }
    if offset < 0 {
        return Err(AccountActivityServiceError::InvalidPagination(
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
    fn test_validate_account_code() {
        assert!(validate_account_code("1000").is_ok());
        assert!(validate_account_code("").is_err());
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
