//! Payment run domain types, errors, and validation.
//!
//! A payment run is a batch proposal of vendor payments.
//! Lifecycle: pending → executing → completed | failed
//!
//! Creation (this module) selects eligible bills and builds the run plan.
//! Execution (bd-295k) instructs the Payments module and records allocations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod builder;
pub mod execute;

// ============================================================================
// Domain types
// ============================================================================

/// Payment run header row from `payment_runs`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentRun {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub total_minor: i64,
    pub currency: String,
    pub scheduled_date: DateTime<Utc>,
    pub payment_method: String,
    pub status: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub executed_at: Option<DateTime<Utc>>,
}

/// Per-vendor item within a payment run from `payment_run_items`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentRunItemRow {
    pub id: i64,
    pub run_id: Uuid,
    pub vendor_id: Uuid,
    pub bill_ids: Vec<Uuid>,
    pub amount_minor: i64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
}

/// Request to create a new payment run.
///
/// The builder selects eligible bills (approved/partially_paid with open balance > 0)
/// for the given tenant and currency, grouped by vendor.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePaymentRunRequest {
    /// Stable idempotency key — same run_id returns the existing run unchanged.
    pub run_id: Uuid,
    /// ISO 4217 currency (single-currency runs only).
    pub currency: String,
    /// Scheduled execution date (when funds should move).
    pub scheduled_date: DateTime<Utc>,
    /// "ach", "wire", or "check"
    pub payment_method: String,
    /// Identity of the user or service creating the run.
    pub created_by: String,
    /// Optional: include only bills due on or before this date.
    pub due_on_or_before: Option<DateTime<Utc>>,
    /// Optional: restrict to these specific vendors.
    pub vendor_ids: Option<Vec<Uuid>>,
    /// Correlation ID for distributed tracing.
    pub correlation_id: Option<String>,
}

impl CreatePaymentRunRequest {
    pub fn validate(&self) -> Result<(), PaymentRunError> {
        let c = self.currency.trim();
        if c.len() != 3 || !c.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return Err(PaymentRunError::Validation(format!(
                "currency must be a 3-letter ISO 4217 code (e.g. USD), got {:?}",
                self.currency
            )));
        }
        if !matches!(self.payment_method.as_str(), "ach" | "wire" | "check") {
            return Err(PaymentRunError::Validation(format!(
                "payment_method must be 'ach', 'wire', or 'check', got {:?}",
                self.payment_method
            )));
        }
        if self.created_by.trim().is_empty() {
            return Err(PaymentRunError::Validation(
                "created_by must not be blank".to_string(),
            ));
        }
        Ok(())
    }
}

/// Returned after successful run creation.
#[derive(Debug)]
pub struct PaymentRunResult {
    pub run: PaymentRun,
    pub items: Vec<PaymentRunItemRow>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum PaymentRunError {
    #[error("no eligible bills found for tenant {0} in currency {1}")]
    NoBillsEligible(String, String),

    #[error("payment run {0} already exists")]
    DuplicateRunId(Uuid),

    #[error("payment run {0} not found")]
    RunNotFound(Uuid),

    #[error("payment run cannot be executed in status '{0}'")]
    RunNotPending(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<PaymentRunError> for platform_http_contracts::ApiError {
    fn from(err: PaymentRunError) -> Self {
        match err {
            PaymentRunError::NoBillsEligible(tenant, currency) => Self::new(
                422,
                "no_eligible_bills",
                format!(
                    "No eligible bills found for tenant '{}' in currency '{}'",
                    tenant, currency
                ),
            ),
            PaymentRunError::DuplicateRunId(id) => Self::conflict(format!(
                "Payment run {} already exists for a different tenant",
                id
            )),
            PaymentRunError::RunNotFound(id) => {
                Self::not_found(format!("Payment run {} not found", id))
            }
            PaymentRunError::RunNotPending(status) => Self::conflict(format!(
                "Payment run cannot be executed in status '{}'",
                status
            )),
            PaymentRunError::Validation(msg) => Self::bad_request(msg),
            PaymentRunError::Database(e) => {
                tracing::error!(error = %e, "Database error in payment run handler");
                Self::internal("An internal error occurred")
            }
        }
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_req() -> CreatePaymentRunRequest {
        CreatePaymentRunRequest {
            run_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            scheduled_date: Utc::now(),
            payment_method: "ach".to_string(),
            created_by: "user-1".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: None,
        }
    }

    #[test]
    fn valid_request_passes() {
        assert!(base_req().validate().is_ok());
    }

    #[test]
    fn valid_payment_methods() {
        for method in ["ach", "wire", "check"] {
            let mut req = base_req();
            req.payment_method = method.to_string();
            assert!(req.validate().is_ok(), "method {} should be valid", method);
        }
    }

    #[test]
    fn invalid_currency_rejected() {
        let mut req = base_req();
        req.currency = "US".to_string();
        assert!(matches!(
            req.validate(),
            Err(PaymentRunError::Validation(_))
        ));
    }

    #[test]
    fn invalid_payment_method_rejected() {
        let mut req = base_req();
        req.payment_method = "crypto".to_string();
        assert!(matches!(
            req.validate(),
            Err(PaymentRunError::Validation(_))
        ));
    }

    #[test]
    fn blank_created_by_rejected() {
        let mut req = base_req();
        req.created_by = "  ".to_string();
        assert!(matches!(
            req.validate(),
            Err(PaymentRunError::Validation(_))
        ));
    }
}
