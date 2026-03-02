//! Event contract for inventory.low_stock_triggered.
//!
//! Emitted (via inv_outbox) when an item's available stock crosses below its
//! configured reorder_point.  The signal is deduped per (item, location) — see
//! `domain/reorder/evaluator.rs` for the crossing-detection logic.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::{create_inventory_envelope, MUTATION_CLASS_DATA_MUTATION};
use event_bus::EventEnvelope;

/// Canonical event type string for the low-stock signal.
pub const EVENT_TYPE_LOW_STOCK_TRIGGERED: &str = "inventory.low_stock_triggered";

// ============================================================================
// Payload
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowStockTriggeredPayload {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Location where the threshold crossing was observed.
    /// None = global (null-location) policy.
    pub location_id: Option<Uuid>,
    /// The reorder_point threshold from the matching policy.
    pub reorder_point: i64,
    /// Current available qty at the time of the crossing.
    pub available_qty: i64,
    pub triggered_at: DateTime<Utc>,
}

// ============================================================================
// Envelope builder
// ============================================================================

pub fn build_low_stock_triggered_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: LowStockTriggeredPayload,
) -> EventEnvelope<LowStockTriggeredPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_LOW_STOCK_TRIGGERED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_constant_matches_spec() {
        assert_eq!(
            EVENT_TYPE_LOW_STOCK_TRIGGERED,
            "inventory.low_stock_triggered"
        );
    }

    #[test]
    fn payload_serializes_with_null_location() {
        let payload = LowStockTriggeredPayload {
            tenant_id: "t1".into(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            reorder_point: 50,
            available_qty: 30,
            triggered_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("low_stock") || json.contains("reorder_point"));
        assert!(json.contains("\"reorder_point\":50"));
        assert!(json.contains("\"available_qty\":30"));
    }

    #[test]
    fn envelope_has_correct_source_module() {
        let payload = LowStockTriggeredPayload {
            tenant_id: "t1".into(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            reorder_point: 10,
            available_qty: 5,
            triggered_at: Utc::now(),
        };
        let env = build_low_stock_triggered_envelope(
            Uuid::new_v4(),
            "t1".into(),
            "corr-1".into(),
            None,
            payload,
        );
        assert_eq!(env.source_module, "inventory");
        assert_eq!(env.event_type, EVENT_TYPE_LOW_STOCK_TRIGGERED);
    }
}
