//! Purchase Order bounded context — types, status machine, validation.
//!
//! POs are commitments to a vendor, not accounting events. They start in
//! draft status and progress through an explicit state machine.
//!
//! Status machine (DB-supported states):
//!   draft      -> approved  (approval/issuance gating — bd-1r7e)
//!   approved   -> closed    (all lines fully received)
//!   draft      -> cancelled
//!   approved   -> cancelled
//!
//! "issued" and "received" sub-states will be tracked via receipt events
//! in bd-1r7e. The conceptual lifecycle is:
//!   draft -> approved -> (issued) -> (received) -> closed
//!
//! Vendor validation is ID-only (no cross-DB reads).
//! Item IDs in line requests are validated as UUIDs; stored as description.

pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum PoError {
    #[error("Purchase order not found: {0}")]
    NotFound(Uuid),

    #[error("Vendor not found or inactive: {0}")]
    VendorNotFound(Uuid),

    #[error("PO cannot be edited; current status: {0}")]
    NotDraft(String),

    #[error("Invalid status transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("PO must have at least one line")]
    EmptyLines,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Status Machine
// ============================================================================

/// PO lifecycle status.
///
/// Permitted transitions:
///   Draft     -> Approved | Cancelled
///   Approved  -> Closed   | Cancelled
///   Closed    -> (terminal)
///   Cancelled -> (terminal)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoStatus {
    /// Initial editable state
    Draft,
    /// Approved for issuance to vendor
    Approved,
    /// All lines received and matched; PO fully settled
    Closed,
    /// Cancelled before completion
    Cancelled,
}

impl PoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PoStatus::Draft => "draft",
            PoStatus::Approved => "approved",
            PoStatus::Closed => "closed",
            PoStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, PoError> {
        match s {
            "draft" => Ok(PoStatus::Draft),
            "approved" => Ok(PoStatus::Approved),
            "closed" => Ok(PoStatus::Closed),
            "cancelled" => Ok(PoStatus::Cancelled),
            other => Err(PoError::Validation(format!("Unknown PO status: '{}'", other))),
        }
    }

    /// Returns true if the transition self -> next is permitted.
    pub fn can_transition_to(self, next: PoStatus) -> bool {
        matches!(
            (self, next),
            (PoStatus::Draft, PoStatus::Approved)
                | (PoStatus::Draft, PoStatus::Cancelled)
                | (PoStatus::Approved, PoStatus::Closed)
                | (PoStatus::Approved, PoStatus::Cancelled)
        )
    }
}

// ============================================================================
// Domain Structs
// ============================================================================

/// Purchase order header record as returned from the DB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PurchaseOrder {
    pub po_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    /// Human-readable PO number (unique per tenant)
    pub po_number: String,
    /// ISO 4217 currency code
    pub currency: String,
    /// Total PO value in minor currency units
    pub total_minor: i64,
    /// Lifecycle status: draft, approved, closed, cancelled
    pub status: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub expected_delivery_date: Option<DateTime<Utc>>,
}

/// A single PO line as returned from the DB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PoLineRecord {
    pub line_id: Uuid,
    pub po_id: Uuid,
    pub description: String,
    pub quantity: f64,
    pub unit_of_measure: String,
    pub unit_price_minor: i64,
    pub line_total_minor: i64,
    pub gl_account_code: String,
    pub created_at: DateTime<Utc>,
}

/// PO with its line items (for GET and POST 201 responses).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseOrderWithLines {
    #[serde(flatten)]
    pub po: PurchaseOrder,
    pub lines: Vec<PoLineRecord>,
}

// ============================================================================
// Request Types
// ============================================================================

/// A single line in a PO create or update request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePoLineRequest {
    /// Optional inventory item UUID reference.
    /// Validated as non-empty if provided; no cross-DB reads are performed.
    /// When description is absent, stored as "item:{uuid}".
    pub item_id: Option<Uuid>,
    /// Human-readable line description. Required when item_id is absent.
    pub description: Option<String>,
    /// Quantity ordered (must be > 0)
    pub quantity: f64,
    /// Unit of measure (e.g. "each", "kg"); defaults to "each"
    #[serde(default = "default_uom")]
    pub unit_of_measure: String,
    /// Unit price in minor currency units (must be >= 0)
    pub unit_price_minor: i64,
    /// GL expense account code
    #[serde(default)]
    pub gl_account_code: String,
}

fn default_uom() -> String {
    "each".to_string()
}

impl CreatePoLineRequest {
    pub fn validate(&self, idx: usize) -> Result<(), PoError> {
        if self.item_id.is_none() {
            let desc_empty = self.description.as_deref()
                .map(|d| d.trim().is_empty())
                .unwrap_or(true);
            if desc_empty {
                return Err(PoError::Validation(format!(
                    "line[{}]: description required when item_id is absent",
                    idx
                )));
            }
        }
        if self.quantity <= 0.0 {
            return Err(PoError::Validation(format!(
                "line[{}]: quantity must be > 0",
                idx
            )));
        }
        if self.unit_price_minor < 0 {
            return Err(PoError::Validation(format!(
                "line[{}]: unit_price_minor must be >= 0",
                idx
            )));
        }
        Ok(())
    }

    /// Resolve the effective description for DB storage.
    pub fn effective_description(&self) -> String {
        match (&self.description, &self.item_id) {
            (Some(d), _) if !d.trim().is_empty() => d.trim().to_string(),
            (_, Some(id)) => format!("item:{}", id),
            _ => String::new(),
        }
    }

    /// Compute line_total_minor = round(quantity * unit_price_minor).
    pub fn line_total_minor(&self) -> i64 {
        (self.quantity * self.unit_price_minor as f64).round() as i64
    }
}

/// Request body to create a purchase order (always created as draft).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePoRequest {
    pub vendor_id: Uuid,
    /// ISO 4217 currency code
    pub currency: String,
    /// Actor creating the PO (for event attribution and audit)
    pub created_by: String,
    /// Optional expected delivery date
    pub expected_delivery_date: Option<DateTime<Utc>>,
    /// Line items (at least one required)
    pub lines: Vec<CreatePoLineRequest>,
}

impl CreatePoRequest {
    pub fn validate(&self) -> Result<(), PoError> {
        if self.currency.trim().len() != 3 {
            return Err(PoError::Validation(
                "currency must be a 3-character ISO 4217 code".to_string(),
            ));
        }
        if self.created_by.trim().is_empty() {
            return Err(PoError::Validation("created_by cannot be empty".to_string()));
        }
        if self.lines.is_empty() {
            return Err(PoError::EmptyLines);
        }
        for (i, line) in self.lines.iter().enumerate() {
            line.validate(i)?;
        }
        Ok(())
    }
}

/// Request body to replace all lines on a draft PO (idempotent full replacement).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePoLinesRequest {
    /// Actor performing the update (for audit)
    pub updated_by: String,
    /// Full replacement line set (at least one required)
    pub lines: Vec<CreatePoLineRequest>,
}

impl UpdatePoLinesRequest {
    pub fn validate(&self) -> Result<(), PoError> {
        if self.updated_by.trim().is_empty() {
            return Err(PoError::Validation("updated_by cannot be empty".to_string()));
        }
        if self.lines.is_empty() {
            return Err(PoError::EmptyLines);
        }
        for (i, line) in self.lines.iter().enumerate() {
            line.validate(i)?;
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

    // --- Status machine ---

    #[test]
    fn draft_to_approved_allowed() {
        assert!(PoStatus::Draft.can_transition_to(PoStatus::Approved));
    }

    #[test]
    fn draft_to_cancelled_allowed() {
        assert!(PoStatus::Draft.can_transition_to(PoStatus::Cancelled));
    }

    #[test]
    fn approved_to_closed_allowed() {
        assert!(PoStatus::Approved.can_transition_to(PoStatus::Closed));
    }

    #[test]
    fn approved_to_cancelled_allowed() {
        assert!(PoStatus::Approved.can_transition_to(PoStatus::Cancelled));
    }

    #[test]
    fn draft_to_closed_not_allowed() {
        assert!(!PoStatus::Draft.can_transition_to(PoStatus::Closed));
    }

    #[test]
    fn closed_is_terminal() {
        assert!(!PoStatus::Closed.can_transition_to(PoStatus::Draft));
        assert!(!PoStatus::Closed.can_transition_to(PoStatus::Approved));
        assert!(!PoStatus::Closed.can_transition_to(PoStatus::Cancelled));
    }

    #[test]
    fn cancelled_is_terminal() {
        assert!(!PoStatus::Cancelled.can_transition_to(PoStatus::Draft));
        assert!(!PoStatus::Cancelled.can_transition_to(PoStatus::Approved));
    }

    #[test]
    fn status_from_str_roundtrips() {
        for s in ["draft", "approved", "closed", "cancelled"] {
            let status = PoStatus::from_str(s).expect(s);
            assert_eq!(status.as_str(), s);
        }
    }

    #[test]
    fn status_from_str_unknown_errors() {
        assert!(PoStatus::from_str("pending").is_err());
        assert!(PoStatus::from_str("issued").is_err());
    }

    // --- Line validation ---

    fn sample_line() -> CreatePoLineRequest {
        CreatePoLineRequest {
            item_id: None,
            description: Some("Office chairs".to_string()),
            quantity: 10.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 45_000,
            gl_account_code: "6100".to_string(),
        }
    }

    #[test]
    fn line_requires_description_when_no_item_id() {
        let mut line = sample_line();
        line.description = None;
        assert!(line.validate(0).is_err());
    }

    #[test]
    fn line_item_id_alone_is_valid() {
        let line = CreatePoLineRequest {
            item_id: Some(Uuid::new_v4()),
            description: None,
            quantity: 1.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 1_000,
            gl_account_code: "6100".to_string(),
        };
        assert!(line.validate(0).is_ok());
    }

    #[test]
    fn line_effective_description_uses_item_id() {
        let id = Uuid::new_v4();
        let line = CreatePoLineRequest {
            item_id: Some(id),
            description: None,
            quantity: 1.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 1_000,
            gl_account_code: String::new(),
        };
        assert_eq!(line.effective_description(), format!("item:{}", id));
    }

    #[test]
    fn line_description_takes_precedence_over_item_id() {
        let line = CreatePoLineRequest {
            item_id: Some(Uuid::new_v4()),
            description: Some("Custom label".to_string()),
            quantity: 1.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 500,
            gl_account_code: String::new(),
        };
        assert_eq!(line.effective_description(), "Custom label");
    }

    #[test]
    fn line_total_is_qty_times_price() {
        assert_eq!(sample_line().line_total_minor(), 450_000);
    }

    // --- Request validation ---

    fn sample_req() -> CreatePoRequest {
        CreatePoRequest {
            vendor_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            created_by: "user-1".to_string(),
            expected_delivery_date: None,
            lines: vec![sample_line()],
        }
    }

    #[test]
    fn create_req_rejects_bad_currency() {
        let mut req = sample_req();
        req.currency = "US".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_req_rejects_empty_created_by() {
        let mut req = sample_req();
        req.created_by = "  ".to_string();
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_req_rejects_empty_lines() {
        let mut req = sample_req();
        req.lines.clear();
        assert!(matches!(req.validate(), Err(PoError::EmptyLines)));
    }

    #[test]
    fn create_req_accepts_valid() {
        assert!(sample_req().validate().is_ok());
    }
}
