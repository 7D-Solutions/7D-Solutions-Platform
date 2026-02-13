//! Trial Balance Service
//!
//! Provides business logic for generating trial balance reports.
//! Trial balance is a key accounting primitive that shows all account balances
//! at a point in time (accounting period).

use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::repos::balance_repo::{self, BalanceError, TrialBalanceRow};

/// Errors that can occur during trial balance operations
#[derive(Debug, Error)]
pub enum TrialBalanceError {
    #[error("Repository error: {0}")]
    Repository(#[from] BalanceError),

    #[error("Invalid period: {0}")]
    InvalidPeriod(String),
}

/// Query trial balance for a tenant and period
///
/// Returns all account balances with metadata for the specified tenant and period,
/// optionally filtered by currency.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period identifier
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Vector of trial balance rows sorted by account_code, currency
///
/// # Example
/// ```ignore
/// let trial_balance = get_trial_balance(
///     &pool,
///     "tenant_123",
///     period_id,
///     Some("USD"),
/// ).await?;
/// ```
pub async fn get_trial_balance(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<Vec<TrialBalanceRow>, TrialBalanceError> {
    // Fetch trial balance with account metadata
    let rows = balance_repo::find_trial_balance_with_metadata(
        pool,
        tenant_id,
        period_id,
        currency,
    )
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = TrialBalanceError::InvalidPeriod("period not found".to_string());
        assert!(err.to_string().contains("Invalid period"));
    }
}
