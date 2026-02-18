//! Domain models for lot and serial instance tracking.
//!
//! ## Lot tracking
//! A lot groups a quantity of items received together (same batch, origin, expiry).
//! Lot-tracked items must provide a `lot_code` on receipt; the code binds to
//! the resulting FIFO layer via `inventory_layers.lot_id`.
//!
//! ## Serial tracking
//! Each unit of a serial-tracked item has a globally unique `serial_code` within
//! the tenant+item scope. One `InventorySerialInstance` row is created per unit
//! on receipt and transitions through a terminal lifecycle (on_hand → issued/
//! transferred/adjusted).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Lot models
// ============================================================================

/// A named lot grouping a batch of items received together.
///
/// Lots are unique per (tenant_id, item_id, lot_code).
/// Created on receipt; immutable thereafter (lot_code and item_id never change).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InventoryLot {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    /// Human-readable lot identifier (e.g. "LOT-2026-001", supplier batch code).
    pub lot_code: String,
    /// Optional free-form metadata stored as JSON (e.g. expiry date, supplier info).
    pub attributes: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a new lot (typically via receipt service).
#[derive(Debug, Clone)]
pub struct CreateLotRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub lot_code: String,
    pub attributes: Option<serde_json::Value>,
}

// ============================================================================
// Serial instance models
// ============================================================================

/// Lifecycle state of a serial-tracked unit.
///
/// Transitions are terminal — once a unit leaves `OnHand` it cannot be
/// re-received. The ledger is append-only; compensating movements create new
/// entries rather than reversing this status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SerialStatus {
    OnHand,
    Issued,
    Transferred,
    Adjusted,
}

impl SerialStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OnHand => "on_hand",
            Self::Issued => "issued",
            Self::Transferred => "transferred",
            Self::Adjusted => "adjusted",
        }
    }
}

impl std::fmt::Display for SerialStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for SerialStatus {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "on_hand" => Ok(Self::OnHand),
            "issued" => Ok(Self::Issued),
            "transferred" => Ok(Self::Transferred),
            "adjusted" => Ok(Self::Adjusted),
            other => Err(format!(
                "invalid serial status '{}': expected on_hand|issued|transferred|adjusted",
                other
            )),
        }
    }
}

/// A single serialised unit received into inventory.
///
/// Created one-per-serial-code when a serial-tracked receipt is processed.
/// Each instance is tied to the receipt ledger entry and FIFO layer.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InventorySerialInstance {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    /// The barcode, manufacturer serial number, or other unique identifier.
    pub serial_code: String,
    /// The receipt ledger entry (`inventory_ledger.id`) that created this instance.
    pub receipt_ledger_entry_id: i64,
    /// The FIFO layer this unit occupies.
    pub layer_id: Uuid,
    /// Current lifecycle state.
    #[sqlx(try_from = "String")]
    pub status: SerialStatus,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a serial instance (one per serial_code in a receipt).
#[derive(Debug, Clone)]
pub struct CreateSerialInstanceRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub serial_code: String,
    pub receipt_ledger_entry_id: i64,
    pub layer_id: Uuid,
}

// ============================================================================
// Unit tests (pure model; DB tests live in integration suite)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_status_roundtrip() {
        for (s, expected) in [
            ("on_hand", SerialStatus::OnHand),
            ("issued", SerialStatus::Issued),
            ("transferred", SerialStatus::Transferred),
            ("adjusted", SerialStatus::Adjusted),
        ] {
            assert_eq!(SerialStatus::try_from(s.to_string()), Ok(expected));
            assert_eq!(expected.as_str(), s);
        }
    }

    #[test]
    fn serial_status_invalid_rejected() {
        assert!(SerialStatus::try_from("unknown".to_string()).is_err());
    }

    #[test]
    fn serial_status_display() {
        assert_eq!(format!("{}", SerialStatus::OnHand), "on_hand");
        assert_eq!(format!("{}", SerialStatus::Issued), "issued");
    }
}
