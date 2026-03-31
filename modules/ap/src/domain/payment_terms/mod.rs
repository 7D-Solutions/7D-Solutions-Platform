//! Payment terms bounded context — types, validation, due-date derivation.
//!
//! Payment terms are structured models (not free-text) that drive deterministic
//! due date and discount date calculations on vendor invoices.
//!
//! Supported term types:
//!   - Net terms: Net 30, Net 60, etc. (days_due only)
//!   - Discount terms: 2/10 Net 30 (discount_pct + discount_days + days_due)
//!   - Installment schedules: JSONB array of {pct, days_due} entries

pub mod service;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum PaymentTermsError {
    #[error("Payment terms not found: {0}")]
    NotFound(Uuid),

    #[error("Bill not found: {0}")]
    BillNotFound(Uuid),

    #[error("Duplicate term_code '{0}' already exists for tenant")]
    DuplicateTermCode(String),

    #[error("Duplicate idempotency_key '{0}'")]
    DuplicateIdempotencyKey(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<PaymentTermsError> for platform_http_contracts::ApiError {
    fn from(err: PaymentTermsError) -> Self {
        match err {
            PaymentTermsError::NotFound(id) => {
                Self::not_found(format!("Payment terms {} not found", id))
            }
            PaymentTermsError::BillNotFound(id) => {
                Self::not_found(format!("Bill {} not found", id))
            }
            PaymentTermsError::DuplicateTermCode(code) => {
                Self::conflict(format!("Term code '{}' already exists", code))
            }
            PaymentTermsError::DuplicateIdempotencyKey(key) => {
                Self::conflict(format!("Idempotency key '{}' already used", key))
            }
            PaymentTermsError::Validation(msg) => Self::new(422, "validation_error", msg),
            PaymentTermsError::Database(e) => {
                tracing::error!("AP payment_terms DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

/// Payment terms record as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct PaymentTerms {
    pub term_id: Uuid,
    pub tenant_id: String,
    pub term_code: String,
    pub description: String,
    pub days_due: i32,
    pub discount_pct: f64,
    pub discount_days: i32,
    pub installment_schedule: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body to create payment terms.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatePaymentTermsRequest {
    pub term_code: String,
    pub description: Option<String>,
    pub days_due: i32,
    /// Discount percentage (e.g. 2.0 for 2%). Defaults to 0.
    pub discount_pct: Option<f64>,
    /// Days within which discount applies. Defaults to 0.
    pub discount_days: Option<i32>,
    /// Installment schedule as a JSON array (optional).
    pub installment_schedule: Option<serde_json::Value>,
    /// Idempotency key for duplicate prevention.
    pub idempotency_key: Option<String>,
}

/// Request body to update payment terms (partial update).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdatePaymentTermsRequest {
    pub description: Option<String>,
    pub days_due: Option<i32>,
    pub discount_pct: Option<f64>,
    pub discount_days: Option<i32>,
    pub installment_schedule: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

/// Request to assign payment terms to an existing bill.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssignTermsRequest {
    pub term_id: Uuid,
}

/// Result of assigning terms to a bill — the computed dates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignTermsResult {
    pub bill_id: Uuid,
    pub term_id: Uuid,
    pub due_date: DateTime<Utc>,
    pub discount_date: Option<DateTime<Utc>>,
    pub discount_amount_minor: Option<i64>,
}

// ============================================================================
// Validation
// ============================================================================

impl CreatePaymentTermsRequest {
    pub fn validate(&self) -> Result<(), PaymentTermsError> {
        if self.term_code.trim().is_empty() {
            return Err(PaymentTermsError::Validation(
                "term_code cannot be empty".to_string(),
            ));
        }
        if self.days_due < 0 {
            return Err(PaymentTermsError::Validation(
                "days_due must be >= 0".to_string(),
            ));
        }
        if let Some(pct) = self.discount_pct {
            if !(0.0..=100.0).contains(&pct) {
                return Err(PaymentTermsError::Validation(
                    "discount_pct must be between 0 and 100".to_string(),
                ));
            }
        }
        if let Some(days) = self.discount_days {
            if days < 0 {
                return Err(PaymentTermsError::Validation(
                    "discount_days must be >= 0".to_string(),
                ));
            }
            if days > self.days_due {
                return Err(PaymentTermsError::Validation(
                    "discount_days cannot exceed days_due".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl UpdatePaymentTermsRequest {
    pub fn validate(&self) -> Result<(), PaymentTermsError> {
        if let Some(days) = self.days_due {
            if days < 0 {
                return Err(PaymentTermsError::Validation(
                    "days_due must be >= 0".to_string(),
                ));
            }
        }
        if let Some(pct) = self.discount_pct {
            if !(0.0..=100.0).contains(&pct) {
                return Err(PaymentTermsError::Validation(
                    "discount_pct must be between 0 and 100".to_string(),
                ));
            }
        }
        if let Some(days) = self.discount_days {
            if days < 0 {
                return Err(PaymentTermsError::Validation(
                    "discount_days must be >= 0".to_string(),
                ));
            }
        }
        Ok(())
    }
}

// ============================================================================
// Due Date Computation
// ============================================================================

/// Compute the payment due date from invoice date and payment terms.
///
/// Pure function — deterministic given the same inputs.
pub fn compute_due_date(invoice_date: NaiveDate, days_due: i32) -> NaiveDate {
    invoice_date + chrono::Duration::days(days_due.max(0) as i64)
}

/// Compute the early-payment discount date.
///
/// Returns None if discount_days is 0 (no discount applies).
pub fn compute_discount_date(invoice_date: NaiveDate, discount_days: i32) -> Option<NaiveDate> {
    if discount_days <= 0 {
        return None;
    }
    Some(invoice_date + chrono::Duration::days(discount_days as i64))
}

/// Compute the discount amount in minor currency units.
///
/// Returns None if discount_pct is 0.
pub fn compute_discount_amount(total_minor: i64, discount_pct: f64) -> Option<i64> {
    if discount_pct <= 0.0 {
        return None;
    }
    Some(((total_minor as f64) * discount_pct / 100.0).round() as i64)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn compute_due_date_net30() {
        let inv = NaiveDate::from_ymd_opt(2026, 1, 1).expect("static date");
        assert_eq!(
            compute_due_date(inv, 30),
            NaiveDate::from_ymd_opt(2026, 1, 31).expect("static date")
        );
    }

    #[test]
    fn compute_due_date_net60() {
        let inv = NaiveDate::from_ymd_opt(2026, 1, 1).expect("static date");
        assert_eq!(
            compute_due_date(inv, 60),
            NaiveDate::from_ymd_opt(2026, 3, 2).expect("static date")
        );
    }

    #[test]
    fn compute_due_date_net0_same_day() {
        let inv = NaiveDate::from_ymd_opt(2026, 3, 15).expect("static date");
        assert_eq!(compute_due_date(inv, 0), inv);
    }

    #[test]
    fn compute_discount_date_returns_none_for_zero() {
        let inv = NaiveDate::from_ymd_opt(2026, 1, 1).expect("static date");
        assert!(compute_discount_date(inv, 0).is_none());
    }

    #[test]
    fn compute_discount_date_10_days() {
        let inv = NaiveDate::from_ymd_opt(2026, 1, 1).expect("static date");
        assert_eq!(
            compute_discount_date(inv, 10),
            Some(NaiveDate::from_ymd_opt(2026, 1, 11).expect("static date"))
        );
    }

    #[test]
    fn compute_discount_amount_2_pct() {
        // 2% of 100000 cents = 2000 cents
        assert_eq!(compute_discount_amount(100_000, 2.0), Some(2000));
    }

    #[test]
    fn compute_discount_amount_zero_pct_returns_none() {
        assert!(compute_discount_amount(100_000, 0.0).is_none());
    }

    #[test]
    fn compute_discount_amount_rounds() {
        // 2% of 33333 = 666.66 → 667
        assert_eq!(compute_discount_amount(33_333, 2.0), Some(667));
    }

    #[test]
    fn validate_rejects_empty_term_code() {
        let req = CreatePaymentTermsRequest {
            term_code: "  ".to_string(),
            description: None,
            days_due: 30,
            discount_pct: None,
            discount_days: None,
            installment_schedule: None,
            idempotency_key: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_days_due() {
        let req = CreatePaymentTermsRequest {
            term_code: "NET30".to_string(),
            description: None,
            days_due: -1,
            discount_pct: None,
            discount_days: None,
            installment_schedule: None,
            idempotency_key: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_discount_days_exceeding_due() {
        let req = CreatePaymentTermsRequest {
            term_code: "BAD".to_string(),
            description: None,
            days_due: 10,
            discount_pct: Some(2.0),
            discount_days: Some(20),
            installment_schedule: None,
            idempotency_key: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_discount_terms() {
        let req = CreatePaymentTermsRequest {
            term_code: "2/10NET30".to_string(),
            description: Some("2% 10 Net 30".to_string()),
            days_due: 30,
            discount_pct: Some(2.0),
            discount_days: Some(10),
            installment_schedule: None,
            idempotency_key: Some("idem-1".to_string()),
        };
        assert!(req.validate().is_ok());
    }
}
