//! Issue request, result, and error types.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    domain::{fifo::FifoError, guards::GuardError, lots_serials::issue::LotSerialError},
    events::contracts::{ConsumedLayer, SourceRef},
};

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/issues
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IssueRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Optional storage location to issue from. When set, availability is
    /// checked against this location's on-hand projection. When absent,
    /// availability is derived from warehouse-level FIFO layers (existing behavior).
    #[serde(default)]
    pub location_id: Option<Uuid>,
    /// Quantity to issue (must be > 0 for None/Lot-tracked items).
    /// For Serial-tracked items, quantity is derived from serial_codes.len().
    pub quantity: i64,
    pub currency: String,
    // Source reference (maps to SourceRef in event payload)
    pub source_module: String,
    pub source_type: String,
    pub source_id: String,
    pub source_line_id: Option<String>,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    /// UoM id for the input `quantity`. When present, `quantity` is in this unit
    /// and will be converted to the item's base_uom before writing to the ledger.
    /// When absent, `quantity` is assumed to already be in base_uom units.
    #[serde(default)]
    pub uom_id: Option<Uuid>,
    /// Required for Lot-tracked items. FIFO consumption is restricted to this lot only.
    #[serde(default)]
    pub lot_code: Option<String>,
    /// Required for Serial-tracked items. Each code must be on_hand for this item.
    /// Quantity is derived from this list; the `quantity` field is ignored.
    #[serde(default)]
    pub serial_codes: Option<Vec<String>>,
}

/// Result returned on successful or replayed issue
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IssueResult {
    /// Stable business key for this issue (= ledger entry_id)
    pub issue_line_id: Uuid,
    /// BIGSERIAL ledger row id
    pub ledger_entry_id: i64,
    /// Event id used in outbox
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    #[serde(default)]
    pub location_id: Option<Uuid>,
    pub quantity: i64,
    pub total_cost_minor: i64,
    pub currency: String,
    pub consumed_layers: Vec<ConsumedLayer>,
    pub source_ref: SourceRef,
    pub issued_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum IssueError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Insufficient stock: requested {requested}, available {available}")]
    InsufficientQuantity { requested: i64, available: i64 },

    #[error("FIFO engine error: {0}")]
    Fifo(#[from] FifoError),

    #[error("No stock layers found for this item/warehouse")]
    NoLayersAvailable,

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("lot_code is required for lot-tracked items")]
    LotRequired,

    #[error("Lot '{0}' not found for this item/tenant")]
    LotNotFound(String),

    #[error("serial_codes is required (non-empty) for serial-tracked items")]
    SerialRequired,

    #[error("Serial '{0}' is not available (not found or not on_hand)")]
    SerialNotAvailable(String),
}

impl From<LotSerialError> for IssueError {
    fn from(e: LotSerialError) -> Self {
        match e {
            LotSerialError::SerialNotAvailable(code) => IssueError::SerialNotAvailable(code),
            LotSerialError::Database(e) => IssueError::Database(e),
        }
    }
}

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(super) struct LedgerRow {
    pub(super) id: i64,
    pub(super) entry_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub(super) struct IdempotencyRecord {
    pub(super) response_body: String,
    pub(super) request_hash: String,
}

#[derive(sqlx::FromRow)]
pub(super) struct LayerRow {
    pub(super) id: Uuid,
    pub(super) quantity_remaining: i64,
    pub(super) unit_cost_minor: i64,
}

// ============================================================================
// Unit tests (stateless validation only)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::super::service::validate_request;
    use super::*;

    fn valid_req() -> IssueRequest {
        IssueRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity: 5,
            currency: "usd".to_string(),
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-001".to_string(),
            source_line_id: None,
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        }
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_negative_quantity() {
        let mut r = valid_req();
        r.quantity = -1;
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_currency() {
        let mut r = valid_req();
        r.currency = "".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_source_id() {
        let mut r = valid_req();
        r.source_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }

    #[test]
    fn validate_skips_quantity_check_for_serial_items() {
        let mut r = valid_req();
        r.quantity = 0; // would normally fail
        r.serial_codes = Some(vec!["SN-001".to_string()]);
        // serial_codes present => quantity check skipped => should pass
        assert!(validate_request(&r).is_ok());
    }
}
