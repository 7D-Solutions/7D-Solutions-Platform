//! Adjustment request, result, and error types.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::guards::GuardError;

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/adjustments
#[derive(Debug, Serialize, Deserialize)]
pub struct AdjustRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Optional storage location within the warehouse (bin, shelf, zone).
    /// When absent, the adjustment is location-agnostic — existing behavior.
    #[serde(default)]
    pub location_id: Option<Uuid>,
    /// Signed quantity change (positive = gain, negative = shrinkage/write-off).
    /// Must not be zero.
    pub quantity_delta: i64,
    /// Human-readable reason for the adjustment (required).
    /// Examples: "shrinkage", "cycle_count_correction", "damaged_write_off"
    pub reason: String,
    /// When true, allows a negative delta even if it would drive on_hand below zero.
    /// Default: false (no-negative policy enforced).
    #[serde(default)]
    pub allow_negative: bool,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed adjustment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjustResult {
    /// Stable business key for this adjustment (inv_adjustments.id)
    pub adjustment_id: Uuid,
    /// BIGSERIAL ledger row id
    pub ledger_entry_id: i64,
    /// Outbox event id
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    #[serde(default)]
    pub location_id: Option<Uuid>,
    pub quantity_delta: i64,
    pub reason: String,
    pub adjusted_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum AdjustError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Insufficient on-hand stock: have {available}, adjustment would reduce to {would_be}")]
    NegativeOnHand { available: i64, would_be: i64 },

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(super) struct IdempotencyRecord {
    pub(super) response_body: String,
    pub(super) request_hash: String,
}

#[derive(sqlx::FromRow)]
pub(super) struct LedgerInserted {
    pub(super) id: i64,
}

#[derive(sqlx::FromRow)]
pub(super) struct OnHandRow {
    pub(super) quantity_on_hand: i64,
}

// ============================================================================
// Validation
// ============================================================================

pub(super) fn validate_request(req: &AdjustRequest) -> Result<(), AdjustError> {
    if req.tenant_id.trim().is_empty() {
        return Err(AdjustError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.quantity_delta == 0 {
        return Err(AdjustError::Guard(GuardError::Validation(
            "quantity_delta must not be zero".to_string(),
        )));
    }
    if req.reason.trim().is_empty() {
        return Err(AdjustError::Guard(GuardError::Validation(
            "reason is required".to_string(),
        )));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(AdjustError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_req() -> AdjustRequest {
        AdjustRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity_delta: 10,
            reason: "cycle_count_correction".to_string(),
            allow_negative: false,
            idempotency_key: "adj-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn validate_rejects_zero_delta() {
        let mut r = valid_req();
        r.quantity_delta = 0;
        assert!(matches!(
            validate_request(&r),
            Err(AdjustError::Guard(GuardError::Validation(_)))
        ));
    }

    #[test]
    fn validate_rejects_empty_reason() {
        let mut r = valid_req();
        r.reason = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(AdjustError::Guard(GuardError::Validation(_)))
        ));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(AdjustError::Guard(GuardError::Validation(_)))
        ));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = " ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(AdjustError::Guard(GuardError::Validation(_)))
        ));
    }

    #[test]
    fn validate_accepts_positive_delta() {
        let r = valid_req();
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn validate_accepts_negative_delta() {
        let mut r = valid_req();
        r.quantity_delta = -5;
        assert!(validate_request(&r).is_ok());
    }
}
