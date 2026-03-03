//! Event contract: inventory.valuation_run_completed
//!
//! Emitted when a valuation run completes — covering FIFO, LIFO, WAC,
//! or standard cost method. Each run is a point-in-time valuation of
//! inventory for a tenant's warehouse under a specific method.
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

pub const EVENT_TYPE_VALUATION_RUN_COMPLETED: &str = "inventory.valuation_run_completed";

// ============================================================================
// Payload
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationRunCompletedLine {
    pub item_id: Uuid,
    pub quantity_on_hand: i64,
    pub unit_cost_minor: i64,
    pub total_value_minor: i64,
    pub variance_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationRunCompletedPayload {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub method: String,
    pub as_of: DateTime<Utc>,
    pub total_value_minor: i64,
    pub total_cogs_minor: i64,
    pub currency: String,
    pub line_count: usize,
    pub lines: Vec<ValuationRunCompletedLine>,
}

// ============================================================================
// Envelope builder
// ============================================================================

pub fn build_valuation_run_completed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ValuationRunCompletedPayload,
) -> EventEnvelope<ValuationRunCompletedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VALUATION_RUN_COMPLETED.to_string(),
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
        let payload = ValuationRunCompletedPayload {
            run_id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            method: "lifo".to_string(),
            as_of: Utc::now(),
            total_value_minor: 10_000,
            total_cogs_minor: 5_000,
            currency: "usd".to_string(),
            line_count: 1,
            lines: vec![ValuationRunCompletedLine {
                item_id: Uuid::new_v4(),
                quantity_on_hand: 10,
                unit_cost_minor: 1_000,
                total_value_minor: 10_000,
                variance_minor: 0,
            }],
        };
        let envelope = build_valuation_run_completed_envelope(
            Uuid::new_v4(),
            "t1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_VALUATION_RUN_COMPLETED);
        assert_eq!(envelope.source_module, "inventory");
    }
}
