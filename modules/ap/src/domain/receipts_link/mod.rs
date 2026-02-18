//! Receipt linkage domain — AP view of goods received against PO lines.
//!
//! AP records linkage between a PO line and an external receipt/GRN identifier
//! (sourced from inventory.item_received events or an explicit AP endpoint).
//! This table is the 3-way match anchor: PO line → receipt → vendor bill.
//!
//! Idempotency: enforced by UNIQUE (po_line_id, receipt_id) in po_receipt_links.
//! No cross-module DB writes: only the AP DB is mutated.

pub mod service;

use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum ReceiptLinkError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Request Type
// ============================================================================

/// Request to ingest a goods receipt link into AP's po_receipt_links table.
///
/// All fields are required. Callers must resolve po_line_id before calling the service.
/// For inventory.item_received events with a single-line PO, po_line_id can be
/// inferred by the consumer from AP's own PO data.
#[derive(Debug, Clone)]
pub struct IngestReceiptLinkRequest {
    pub po_id: Uuid,
    pub po_line_id: Uuid,
    pub vendor_id: Uuid,
    /// External receipt/GRN identifier (receipt_line_id from inventory.item_received)
    pub receipt_id: Uuid,
    /// Quantity received on this link (must be > 0)
    pub quantity_received: f64,
    pub unit_of_measure: String,
    /// Unit price from the PO line at creation time (minor currency units)
    pub unit_price_minor: i64,
    /// ISO 4217 currency code
    pub currency: String,
    pub gl_account_code: String,
    pub received_at: DateTime<Utc>,
    /// Actor or system that recorded this receipt (e.g. "system:inventory-consumer")
    pub received_by: String,
}

impl IngestReceiptLinkRequest {
    pub fn validate(&self) -> Result<(), ReceiptLinkError> {
        if self.quantity_received <= 0.0 {
            return Err(ReceiptLinkError::Validation(
                "quantity_received must be > 0".to_string(),
            ));
        }
        if self.unit_price_minor < 0 {
            return Err(ReceiptLinkError::Validation(
                "unit_price_minor must be >= 0".to_string(),
            ));
        }
        if self.currency.trim().len() != 3 {
            return Err(ReceiptLinkError::Validation(
                "currency must be a 3-character ISO 4217 code".to_string(),
            ));
        }
        if self.received_by.trim().is_empty() {
            return Err(ReceiptLinkError::Validation(
                "received_by cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_req() -> IngestReceiptLinkRequest {
        IngestReceiptLinkRequest {
            po_id: Uuid::new_v4(),
            po_line_id: Uuid::new_v4(),
            vendor_id: Uuid::new_v4(),
            receipt_id: Uuid::new_v4(),
            quantity_received: 5.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 1000,
            currency: "USD".to_string(),
            gl_account_code: "6100".to_string(),
            received_at: Utc::now(),
            received_by: "actor".to_string(),
        }
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(sample_req().validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut req = sample_req();
        req.quantity_received = 0.0;
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_quantity() {
        let mut req = sample_req();
        req.quantity_received = -1.0;
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_currency() {
        let mut req = sample_req();
        req.currency = "US".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_received_by() {
        let mut req = sample_req();
        req.received_by = "  ".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_unit_price() {
        let mut req = sample_req();
        req.unit_price_minor = -1;
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_zero_unit_price() {
        let mut req = sample_req();
        req.unit_price_minor = 0;
        assert!(req.validate().is_ok());
    }
}
