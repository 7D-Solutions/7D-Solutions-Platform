//! Period Summary Service
//!
//! Provides read-only period summary reporting backed by period_summary_snapshots
//! with fallback to account_balances. Avoids journal_lines scans for performance.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::repos::period_summary_repo::{self, PeriodSummaryError as RepoError};

/// Period summary response DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeriodSummaryResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String,
    pub journal_count: i32,
    pub line_count: i32,
    pub total_debits_minor: i64,
    pub total_credits_minor: i64,
    pub is_balanced: bool,
    pub data_source: String, // "snapshot" or "computed"
    pub snapshot_created_at: Option<String>, // ISO 8601 timestamp if from snapshot
}

/// Errors that can occur during period summary operations
#[derive(Debug, Error)]
pub enum PeriodSummaryServiceError {
    #[error("Repository error: {0}")]
    Repo(#[from] RepoError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Invalid currency: {0}")]
    InvalidCurrency(String),
}

/// Get period summary for a tenant and period
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Period summary response with counts, totals, and data source
///
/// # Example
/// ```ignore
/// let summary = get_period_summary(
///     &pool,
///     "tenant_123",
///     period_id,
///     Some("USD"),
/// ).await?;
/// ```
pub async fn get_period_summary(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<PeriodSummaryResponse, PeriodSummaryServiceError> {
    // Validate inputs
    if tenant_id.is_empty() {
        return Err(PeriodSummaryServiceError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }

    if let Some(cur) = currency {
        validate_currency(cur)?;
    }

    // Query period summary from repository
    let summary = period_summary_repo::find_period_summary(pool, tenant_id, period_id, currency)
        .await?;

    // Check if balanced
    let is_balanced = summary.total_debits_minor == summary.total_credits_minor;

    // Determine data source
    let data_source = if summary.is_snapshot {
        "snapshot".to_string()
    } else {
        "computed".to_string()
    };

    // Format snapshot timestamp if present
    let snapshot_created_at = summary
        .snapshot_created_at
        .map(|dt| dt.to_rfc3339());

    Ok(PeriodSummaryResponse {
        tenant_id: summary.tenant_id,
        period_id: summary.period_id,
        currency: summary.currency,
        journal_count: summary.journal_count,
        line_count: summary.line_count,
        total_debits_minor: summary.total_debits_minor,
        total_credits_minor: summary.total_credits_minor,
        is_balanced,
        data_source,
        snapshot_created_at,
    })
}

/// Validate currency code (ISO 4217: 3 uppercase letters)
fn validate_currency(currency: &str) -> Result<(), PeriodSummaryServiceError> {
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(PeriodSummaryServiceError::InvalidCurrency(format!(
            "Currency must be 3 uppercase letters (ISO 4217): {}",
            currency
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_currency_valid() {
        assert!(validate_currency("USD").is_ok());
        assert!(validate_currency("EUR").is_ok());
        assert!(validate_currency("GBP").is_ok());
    }

    #[test]
    fn test_validate_currency_invalid() {
        assert!(validate_currency("usd").is_err()); // lowercase
        assert!(validate_currency("US").is_err()); // too short
        assert!(validate_currency("USDD").is_err()); // too long
        assert!(validate_currency("U$D").is_err()); // special char
        assert!(validate_currency("").is_err()); // empty
    }

    #[test]
    fn test_period_summary_response() {
        let response = PeriodSummaryResponse {
            tenant_id: "tenant_123".to_string(),
            period_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            journal_count: 10,
            line_count: 20,
            total_debits_minor: 100000,
            total_credits_minor: 100000,
            is_balanced: true,
            data_source: "snapshot".to_string(),
            snapshot_created_at: Some("2024-01-01T00:00:00Z".to_string()),
        };

        assert_eq!(response.tenant_id, "tenant_123");
        assert_eq!(response.journal_count, 10);
        assert!(response.is_balanced);
        assert_eq!(response.data_source, "snapshot");
    }
}
