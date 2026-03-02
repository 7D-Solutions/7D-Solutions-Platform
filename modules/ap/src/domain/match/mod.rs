//! 3-way match bounded context — types, request/response, error handling.
//!
//! The match engine compares bill lines to PO lines and received quantities
//! (from AP receipt links). Results are stored as append-only match records
//! in three_way_match. No auto-approval; the engine only computes and stores
//! match status + variances.
//!
//! Match types:
//!   three_way — PO ↔ Receipt ↔ Bill (full verification)
//!   two_way   — PO ↔ Bill (no receipt available)
//!   non_po    — Bill only (no PO backing, spot purchase)
//!
//! Tolerance defaults:
//!   price: 5% of PO unit_price × matched_qty (configurable per request)
//!   qty:   exact match (0 tolerance)

pub mod engine;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum MatchError {
    #[error("Bill not found: {0}")]
    BillNotFound(Uuid),

    #[error("PO not found: {0}")]
    PoNotFound(Uuid),

    #[error("Bill status '{0}' cannot be matched; must be 'open' or 'matched'")]
    InvalidBillStatus(String),

    #[error("Bill has no lines")]
    NoMatchableLines,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Match Status
// ============================================================================

/// Per-line match outcome classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    /// Qty and price both within tolerance
    Matched,
    /// Qty OK, price outside tolerance
    PriceVariance,
    /// Price OK, qty outside tolerance
    QtyVariance,
    /// Both qty and price outside tolerance
    PriceAndQtyVariance,
}

impl MatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchStatus::Matched => "matched",
            MatchStatus::PriceVariance => "price_variance",
            MatchStatus::QtyVariance => "qty_variance",
            MatchStatus::PriceAndQtyVariance => "price_and_qty_variance",
        }
    }
}

// ============================================================================
// Request
// ============================================================================

/// Request to run the match engine for a vendor bill against a PO.
#[derive(Debug, Clone, Deserialize)]
pub struct RunMatchRequest {
    /// PO to match this bill against
    pub po_id: Uuid,
    /// Actor triggering the match (for audit trail)
    pub matched_by: String,
    /// Price tolerance as a fraction (0.0–1.0). Default: 0.05 (5%).
    /// Variance ≤ po_price × matched_qty × tolerance is within tolerance.
    #[serde(default = "default_price_tolerance")]
    pub price_tolerance_pct: f64,
}

fn default_price_tolerance() -> f64 {
    0.05
}

impl RunMatchRequest {
    pub fn validate(&self) -> Result<(), MatchError> {
        if self.matched_by.trim().is_empty() {
            return Err(MatchError::Validation(
                "matched_by cannot be empty".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&self.price_tolerance_pct) {
            return Err(MatchError::Validation(
                "price_tolerance_pct must be between 0.0 and 1.0".to_string(),
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Per-line result
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct MatchLineResult {
    pub bill_line_id: Uuid,
    /// PO line matched against (None for non-PO lines)
    pub po_line_id: Option<Uuid>,
    /// Primary receipt used (None for two_way or non_po)
    pub receipt_id: Option<Uuid>,
    /// "two_way", "three_way", or "non_po"
    pub match_type: String,
    pub matched_quantity: f64,
    /// Amount matched at PO price (minor currency units)
    pub matched_amount_minor: i64,
    /// (bill_price - po_price) × matched_qty (minor currency units; signed)
    pub price_variance_minor: i64,
    /// bill_qty - (received_qty or po_qty); positive means over-billed
    pub qty_variance: f64,
    pub within_tolerance: bool,
    /// "matched" | "price_variance" | "qty_variance" | "price_and_qty_variance"
    pub match_status: String,
}

// ============================================================================
// Overall outcome
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct MatchOutcome {
    pub bill_id: Uuid,
    pub po_id: Uuid,
    pub lines: Vec<MatchLineResult>,
    /// True when all lines are within tolerance
    pub fully_matched: bool,
    pub matched_by: String,
    pub matched_at: DateTime<Utc>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_status_as_str_roundtrips() {
        assert_eq!(MatchStatus::Matched.as_str(), "matched");
        assert_eq!(MatchStatus::PriceVariance.as_str(), "price_variance");
        assert_eq!(MatchStatus::QtyVariance.as_str(), "qty_variance");
        assert_eq!(
            MatchStatus::PriceAndQtyVariance.as_str(),
            "price_and_qty_variance"
        );
    }

    #[test]
    fn validate_rejects_empty_matched_by() {
        let req = RunMatchRequest {
            po_id: Uuid::new_v4(),
            matched_by: "  ".to_string(),
            price_tolerance_pct: 0.05,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_tolerance_above_one() {
        let req = RunMatchRequest {
            po_id: Uuid::new_v4(),
            matched_by: "user-1".to_string(),
            price_tolerance_pct: 1.5,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_tolerance() {
        let req = RunMatchRequest {
            po_id: Uuid::new_v4(),
            matched_by: "user-1".to_string(),
            price_tolerance_pct: -0.01,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_zero_tolerance() {
        let req = RunMatchRequest {
            po_id: Uuid::new_v4(),
            matched_by: "user-1".to_string(),
            price_tolerance_pct: 0.0,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = RunMatchRequest {
            po_id: Uuid::new_v4(),
            matched_by: "user-1".to_string(),
            price_tolerance_pct: 0.05,
        };
        assert!(req.validate().is_ok());
    }
}
