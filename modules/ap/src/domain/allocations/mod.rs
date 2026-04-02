//! AP allocations bounded context — append-only payment application to vendor bills.
//!
//! Allocations record how payments are applied against approved/partially_paid bills.
//! The ap_allocations table is APPEND-ONLY: no UPDATE, no DELETE.
//!
//! ## Status machine (bill side)
//!
//! ```text
//! approved  ──(partial allocation)──► partially_paid
//! approved  ──(full allocation)──────► paid
//! partially_paid ──(further allocation to zero balance)──► paid
//! ```
//!
//! Status derives deterministically from the sum of allocations vs bill total_minor.
//!
//! ## Idempotency
//!
//! `allocation_id` (UUID) is the stable anchor: caller-generated, unique per allocation.
//! Duplicate submissions with the same allocation_id return the existing record.

pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum AllocationError {
    #[error("Bill not found: {0}")]
    BillNotFound(Uuid),

    #[error("Bill status '{0}' does not permit allocation (must be approved or partially_paid)")]
    InvalidBillStatus(String),

    #[error("Allocation would exceed bill balance: available={available}, requested={requested}")]
    OverAllocation { available: i64, requested: i64 },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<AllocationError> for platform_http_contracts::ApiError {
    fn from(err: AllocationError) -> Self {
        match err {
            AllocationError::BillNotFound(id) => {
                Self::not_found(format!("Bill {} not found", id))
            }
            AllocationError::InvalidBillStatus(status) => Self::new(
                422,
                "invalid_bill_status",
                format!(
                    "Bill status '{}' does not accept allocations; \
                     bill must be 'approved' or 'partially_paid'",
                    status
                ),
            ),
            AllocationError::OverAllocation {
                available,
                requested,
            } => Self::new(
                422,
                "over_allocation",
                format!(
                    "Allocation of {} would exceed open balance of {}",
                    requested, available
                ),
            ),
            AllocationError::Validation(msg) => Self::bad_request(msg),
            AllocationError::Database(e) => {
                tracing::error!(error = %e, "Database error in allocation handler");
                Self::internal("An internal error occurred")
            }
        }
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

/// A single allocation record as stored in ap_allocations.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct AllocationRecord {
    /// BIGSERIAL primary key (internal ordering)
    pub id: i64,
    /// Stable external idempotency anchor — caller-generated UUID.
    pub allocation_id: Uuid,
    pub bill_id: Uuid,
    /// Set when this allocation is claimed by a payment run.
    pub payment_run_id: Option<Uuid>,
    pub tenant_id: String,
    /// Amount applied in minor currency units (always > 0).
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// "partial" or "full" — derived from remaining bill balance at allocation time.
    pub allocation_type: String,
    pub created_at: DateTime<Utc>,
}

/// Summary of remaining open balance on a bill.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BillBalanceSummary {
    pub bill_id: Uuid,
    pub total_minor: i64,
    pub allocated_minor: i64,
    pub open_balance_minor: i64,
    pub status: String,
}

// ============================================================================
// Request Types
// ============================================================================

/// Request body to apply a payment allocation to a vendor bill.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateAllocationRequest {
    /// Caller-generated stable UUID — used for idempotency.
    /// Duplicate submissions with the same allocation_id return the existing record.
    pub allocation_id: Uuid,
    /// Amount to apply in minor currency units (must be > 0).
    pub amount_minor: i64,
    /// ISO 4217 currency code. Must match the bill currency.
    pub currency: String,
    /// Optional: payment run claiming this allocation (may be set later).
    pub payment_run_id: Option<Uuid>,
}

impl CreateAllocationRequest {
    pub fn validate(&self) -> Result<(), AllocationError> {
        if self.amount_minor <= 0 {
            return Err(AllocationError::Validation(
                "amount_minor must be > 0".to_string(),
            ));
        }
        let c = self.currency.trim();
        if c.len() != 3 || !c.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return Err(AllocationError::Validation(
                "currency must be a 3-letter ISO 4217 code (e.g. USD)".to_string(),
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Status Derivation
// ============================================================================

/// Derive the new bill status from total_minor and the post-allocation sum of allocations.
///
/// Only called when transitioning from 'approved' or 'partially_paid'.
/// Returns the correct new status string.
pub fn derive_bill_status(total_minor: i64, allocated_minor: i64) -> &'static str {
    if allocated_minor >= total_minor {
        "paid"
    } else {
        "partially_paid"
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_status_full_payment_is_paid() {
        assert_eq!(derive_bill_status(50000, 50000), "paid");
    }

    #[test]
    fn derive_status_over_payment_is_paid() {
        // Should not happen (guard prevents it) but status derivation is safe
        assert_eq!(derive_bill_status(50000, 60000), "paid");
    }

    #[test]
    fn derive_status_partial_payment_is_partially_paid() {
        assert_eq!(derive_bill_status(50000, 30000), "partially_paid");
    }

    #[test]
    fn validate_rejects_zero_amount() {
        let req = CreateAllocationRequest {
            allocation_id: Uuid::new_v4(),
            amount_minor: 0,
            currency: "USD".to_string(),
            payment_run_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_amount() {
        let req = CreateAllocationRequest {
            allocation_id: Uuid::new_v4(),
            amount_minor: -100,
            currency: "USD".to_string(),
            payment_run_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_invalid_currency() {
        let req = CreateAllocationRequest {
            allocation_id: Uuid::new_v4(),
            amount_minor: 1000,
            currency: "US".to_string(),
            payment_run_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = CreateAllocationRequest {
            allocation_id: Uuid::new_v4(),
            amount_minor: 50000,
            currency: "USD".to_string(),
            payment_run_id: None,
        };
        assert!(req.validate().is_ok());
    }
}
