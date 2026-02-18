//! Bill bounded context — types, validation, and status machine.
//!
//! Vendor bills are AP payable obligations entered against a vendor.
//! Bills can be "unmatched" (non-PO) or matched to a PO via the match engine.
//! Due date is derived deterministically from vendor payment terms when not
//! explicitly provided.
//!
//! Status machine: open -> approved -> partially_paid -> paid -> voided
//!                  open -> matched (3-way match engine, future bead)

pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum BillError {
    #[error("Bill not found: {0}")]
    NotFound(Uuid),

    #[error("Vendor not found or inactive: {0}")]
    VendorNotFound(Uuid),

    #[error("Duplicate invoice: '{0}' already exists for this vendor")]
    DuplicateInvoice(String),

    #[error("Invalid status transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Bill must have at least one line")]
    EmptyLines,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Status Machine
// ============================================================================

/// Bill lifecycle status.
///
/// Permitted transitions:
///   open          -> approved | matched | voided
///   matched       -> approved | voided
///   approved      -> partially_paid | paid | voided
///   partially_paid -> paid | voided
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillStatus {
    /// Initial state: bill entered, awaiting approval or matching
    Open,
    /// Bill matched to a PO (2-way or 3-way match engine)
    Matched,
    /// Bill approved for payment
    Approved,
    /// Bill partially paid
    PartiallyPaid,
    /// Bill fully paid
    Paid,
    /// Bill voided (compensating event)
    Voided,
}

impl BillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BillStatus::Open => "open",
            BillStatus::Matched => "matched",
            BillStatus::Approved => "approved",
            BillStatus::PartiallyPaid => "partially_paid",
            BillStatus::Paid => "paid",
            BillStatus::Voided => "voided",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, BillError> {
        match s {
            "open" => Ok(BillStatus::Open),
            "matched" => Ok(BillStatus::Matched),
            "approved" => Ok(BillStatus::Approved),
            "partially_paid" => Ok(BillStatus::PartiallyPaid),
            "paid" => Ok(BillStatus::Paid),
            "voided" => Ok(BillStatus::Voided),
            other => Err(BillError::Validation(format!("Unknown bill status: '{}'", other))),
        }
    }

    /// Returns true if the transition self -> next is permitted.
    pub fn can_transition_to(self, next: BillStatus) -> bool {
        matches!(
            (self, next),
            (BillStatus::Open, BillStatus::Approved)
                | (BillStatus::Open, BillStatus::Matched)
                | (BillStatus::Open, BillStatus::Voided)
                | (BillStatus::Matched, BillStatus::Approved)
                | (BillStatus::Matched, BillStatus::Voided)
                | (BillStatus::Approved, BillStatus::PartiallyPaid)
                | (BillStatus::Approved, BillStatus::Paid)
                | (BillStatus::Approved, BillStatus::Voided)
                | (BillStatus::PartiallyPaid, BillStatus::Paid)
                | (BillStatus::PartiallyPaid, BillStatus::Voided)
        )
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

/// A vendor bill header record as returned from the DB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct VendorBill {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    /// Vendor's external invoice/bill reference number
    pub vendor_invoice_ref: String,
    /// ISO 4217 currency code
    pub currency: String,
    /// Total bill amount in minor currency units
    pub total_minor: i64,
    /// Tax amount in minor currency units (optional)
    pub tax_minor: Option<i64>,
    pub invoice_date: DateTime<Utc>,
    pub due_date: DateTime<Utc>,
    /// Lifecycle status string ("open", "approved", "partially_paid", "paid", "voided", "matched")
    pub status: String,
    /// GL fx_rates UUID — NULL when bill currency == functional currency (no FX needed)
    pub fx_rate_id: Option<Uuid>,
    pub entered_by: String,
    pub entered_at: DateTime<Utc>,
}

/// A single line on a vendor bill as returned from the DB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BillLineRecord {
    pub line_id: Uuid,
    pub bill_id: Uuid,
    pub description: String,
    /// Quantity (DOUBLE PRECISION in DB, f64 in Rust)
    pub quantity: f64,
    pub unit_price_minor: i64,
    pub line_total_minor: i64,
    pub gl_account_code: String,
    pub po_line_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// A vendor bill with its line items (for GET responses and POST 201).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorBillWithLines {
    #[serde(flatten)]
    pub bill: VendorBill,
    pub lines: Vec<BillLineRecord>,
}

// ============================================================================
// Request Types
// ============================================================================

/// A single bill line in the creation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBillLineRequest {
    /// Optional description (defaults to "" if not supplied)
    pub description: Option<String>,
    /// Optional item reference — informational only, not persisted in this bead
    pub item_id: Option<Uuid>,
    pub quantity: f64,
    pub unit_price_minor: i64,
    /// GL account code (defaults to "" if not supplied)
    pub gl_account_code: Option<String>,
    /// PO line reference for matching (None for unmatched/non-PO bills)
    pub po_line_id: Option<Uuid>,
}

/// Request body to create a vendor bill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBillRequest {
    pub vendor_id: Uuid,
    /// Vendor's external invoice reference (must be unique per vendor per tenant)
    pub vendor_invoice_ref: String,
    /// ISO 4217 currency code
    pub currency: String,
    /// Invoice date from the vendor
    pub invoice_date: DateTime<Utc>,
    /// Explicit due date; if None, derived deterministically from vendor payment terms
    pub due_date: Option<DateTime<Utc>>,
    /// Optional tax total in minor currency units
    pub tax_minor: Option<i64>,
    /// Actor entering the bill
    pub entered_by: String,
    /// GL fx_rates UUID — supply when bill currency != tenant functional currency.
    /// Reuses GL FX infrastructure; do not pass a raw rate value here.
    pub fx_rate_id: Option<Uuid>,
    pub lines: Vec<CreateBillLineRequest>,
}

// ============================================================================
// Validation
// ============================================================================

impl CreateBillRequest {
    pub fn validate(&self) -> Result<(), BillError> {
        if self.vendor_invoice_ref.trim().is_empty() {
            return Err(BillError::Validation(
                "vendor_invoice_ref cannot be empty".to_string(),
            ));
        }
        if self.currency.trim().len() != 3 {
            return Err(BillError::Validation(
                "currency must be a 3-character ISO 4217 code".to_string(),
            ));
        }
        if self.entered_by.trim().is_empty() {
            return Err(BillError::Validation("entered_by cannot be empty".to_string()));
        }
        if self.lines.is_empty() {
            return Err(BillError::EmptyLines);
        }
        for (i, line) in self.lines.iter().enumerate() {
            line.validate(i)?;
        }
        Ok(())
    }
}

impl CreateBillLineRequest {
    fn validate(&self, idx: usize) -> Result<(), BillError> {
        if self.quantity <= 0.0 {
            return Err(BillError::Validation(format!(
                "line[{}]: quantity must be > 0",
                idx
            )));
        }
        if self.unit_price_minor < 0 {
            return Err(BillError::Validation(format!(
                "line[{}]: unit_price_minor must be >= 0",
                idx
            )));
        }
        Ok(())
    }

    /// Compute line_total_minor = quantity * unit_price_minor, rounded to nearest integer.
    pub fn line_total_minor(&self) -> i64 {
        (self.quantity * self.unit_price_minor as f64).round() as i64
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_line() -> CreateBillLineRequest {
        CreateBillLineRequest {
            description: Some("Consulting services".to_string()),
            item_id: None,
            quantity: 5.0,
            unit_price_minor: 2000,
            gl_account_code: Some("6200".to_string()),
            po_line_id: None,
        }
    }

    fn sample_create() -> CreateBillRequest {
        CreateBillRequest {
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-001".to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "user-1".to_string(),
            fx_rate_id: None,
            lines: vec![sample_line()],
        }
    }

    #[test]
    fn status_open_to_approved_allowed() {
        assert!(BillStatus::Open.can_transition_to(BillStatus::Approved));
    }

    #[test]
    fn status_open_to_matched_allowed() {
        assert!(BillStatus::Open.can_transition_to(BillStatus::Matched));
    }

    #[test]
    fn status_open_to_voided_allowed() {
        assert!(BillStatus::Open.can_transition_to(BillStatus::Voided));
    }

    #[test]
    fn status_approved_to_partially_paid_allowed() {
        assert!(BillStatus::Approved.can_transition_to(BillStatus::PartiallyPaid));
    }

    #[test]
    fn status_partially_paid_to_paid_allowed() {
        assert!(BillStatus::PartiallyPaid.can_transition_to(BillStatus::Paid));
    }

    #[test]
    fn status_paid_cannot_go_back_to_approved() {
        assert!(!BillStatus::Paid.can_transition_to(BillStatus::Approved));
    }

    #[test]
    fn status_paid_cannot_go_to_open() {
        assert!(!BillStatus::Paid.can_transition_to(BillStatus::Open));
    }

    #[test]
    fn status_voided_cannot_transition_to_anything() {
        assert!(!BillStatus::Voided.can_transition_to(BillStatus::Open));
        assert!(!BillStatus::Voided.can_transition_to(BillStatus::Approved));
        assert!(!BillStatus::Voided.can_transition_to(BillStatus::Paid));
    }

    #[test]
    fn status_from_str_roundtrips() {
        for s in ["open", "matched", "approved", "partially_paid", "paid", "voided"] {
            let status = BillStatus::from_str(s).expect(s);
            assert_eq!(status.as_str(), s);
        }
    }

    #[test]
    fn status_from_str_unknown_errors() {
        assert!(BillStatus::from_str("pending").is_err());
        assert!(BillStatus::from_str("").is_err());
    }

    #[test]
    fn line_total_minor_is_qty_times_price() {
        let line = sample_line(); // qty=5.0, price=2000
        assert_eq!(line.line_total_minor(), 10000);
    }

    #[test]
    fn line_total_minor_rounds() {
        let line = CreateBillLineRequest {
            description: None,
            item_id: None,
            quantity: 3.0,
            unit_price_minor: 333,
            gl_account_code: None,
            po_line_id: None,
        };
        // 3 * 333 = 999.0 exactly
        assert_eq!(line.line_total_minor(), 999);
    }

    #[test]
    fn validate_rejects_empty_invoice_ref() {
        let mut req = sample_create();
        req.vendor_invoice_ref = "  ".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_short_currency() {
        let mut req = sample_create();
        req.currency = "US".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_lines() {
        let mut req = sample_create();
        req.lines.clear();
        assert!(matches!(req.validate(), Err(BillError::EmptyLines)));
    }

    #[test]
    fn validate_rejects_negative_quantity() {
        let mut req = sample_create();
        req.lines[0].quantity = -1.0;
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_unit_price() {
        let mut req = sample_create();
        req.lines[0].unit_price_minor = -1;
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(sample_create().validate().is_ok());
    }
}
