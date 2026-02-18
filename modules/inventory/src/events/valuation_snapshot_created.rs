//! Event contract: inventory.valuation_snapshot_created
//!
//! Emitted when a valuation snapshot is built from FIFO layer state
//! as-of a given timestamp. Downstream consumers may use this for
//! audit trails; it does NOT trigger GL close entries.
//!
//! Idempotency: caller MUST supply a deterministic event_id.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constant
// ============================================================================

/// A valuation snapshot was created from FIFO layer state.
pub const EVENT_TYPE_VALUATION_SNAPSHOT_CREATED: &str =
    "inventory.valuation_snapshot_created";

// ============================================================================
// Payload
// ============================================================================

/// One per-item line in the snapshot payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationSnapshotCreatedLine {
    pub item_id: Uuid,
    pub quantity_on_hand: i64,
    /// Weighted-average unit cost across remaining FIFO layers
    pub unit_cost_minor: i64,
    /// quantity_on_hand × unit_cost_minor
    pub total_value_minor: i64,
}

/// Payload for inventory.valuation_snapshot_created
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationSnapshotCreatedPayload {
    pub snapshot_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub location_id: Option<Uuid>,
    pub as_of: DateTime<Utc>,
    pub total_value_minor: i64,
    pub currency: String,
    pub line_count: usize,
    pub lines: Vec<ValuationSnapshotCreatedLine>,
}

// ============================================================================
// Envelope builder
// ============================================================================

pub fn build_valuation_snapshot_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ValuationSnapshotCreatedPayload,
) -> EventEnvelope<ValuationSnapshotCreatedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VALUATION_SNAPSHOT_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INVENTORY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_has_correct_event_type() {
        let payload = ValuationSnapshotCreatedPayload {
            snapshot_id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            as_of: Utc::now(),
            total_value_minor: 10_000,
            currency: "usd".to_string(),
            line_count: 1,
            lines: vec![ValuationSnapshotCreatedLine {
                item_id: Uuid::new_v4(),
                quantity_on_hand: 10,
                unit_cost_minor: 1_000,
                total_value_minor: 10_000,
            }],
        };
        let envelope = build_valuation_snapshot_created_envelope(
            Uuid::new_v4(),
            "t1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_VALUATION_SNAPSHOT_CREATED);
        assert_eq!(envelope.source_module, "inventory");
    }
}
