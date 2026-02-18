//! Event contract: inventory.cycle_count_approved
//!
//! Emitted when a cycle count task is approved. Approval is the boundary where
//! physical count variances become inventory movements (adjustment ledger entries
//! are created for each non-zero variance line).
//!
//! Idempotency: caller MUST supply a deterministic event_id derived from
//! the approve's stable business key (idempotency_key).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constant
// ============================================================================

/// A cycle count task was approved and variances converted to inventory adjustments
pub const EVENT_TYPE_CYCLE_COUNT_APPROVED: &str = "inventory.cycle_count_approved";

// ============================================================================
// Payload
// ============================================================================

/// One line within an approved cycle count (per-item variance and adjustment detail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleCountApprovedLine {
    pub line_id: Uuid,
    pub item_id: Uuid,
    /// Quantity snapshotted at task creation time
    pub expected_qty: i64,
    /// Quantity physically counted by the operator
    pub counted_qty: i64,
    /// Computed: counted_qty − expected_qty (negative = shrinkage, positive = overage)
    pub variance_qty: i64,
    /// None when variance_qty == 0 (no adjustment created)
    pub adjustment_id: Option<Uuid>,
}

/// Payload for inventory.cycle_count_approved
///
/// Emitted when the task moves to 'approved'. Adjustment ledger entries have
/// already been created for all non-zero variance lines before this event fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleCountApprovedPayload {
    pub task_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub location_id: Uuid,
    pub approved_at: DateTime<Utc>,
    pub line_count: usize,
    /// Number of lines where a non-zero variance produced an adjustment
    pub adjustment_count: usize,
    pub lines: Vec<CycleCountApprovedLine>,
}

// ============================================================================
// Envelope builder
// ============================================================================

/// Build an envelope for inventory.cycle_count_approved
pub fn build_cycle_count_approved_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: CycleCountApprovedPayload,
) -> EventEnvelope<CycleCountApprovedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CYCLE_COUNT_APPROVED.to_string(),
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
    use chrono::Utc;

    fn make_payload() -> CycleCountApprovedPayload {
        CycleCountApprovedPayload {
            task_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: Uuid::new_v4(),
            approved_at: Utc::now(),
            line_count: 1,
            adjustment_count: 1,
            lines: vec![CycleCountApprovedLine {
                line_id: Uuid::new_v4(),
                item_id: Uuid::new_v4(),
                expected_qty: 50,
                counted_qty: 45,
                variance_qty: -5,
                adjustment_id: Some(Uuid::new_v4()),
            }],
        }
    }

    #[test]
    fn envelope_has_correct_metadata() {
        let payload = make_payload();
        let envelope = build_cycle_count_approved_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CYCLE_COUNT_APPROVED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn causation_id_propagated() {
        let payload = make_payload();
        let envelope = build_cycle_count_approved_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
    }

    #[test]
    fn zero_variance_line_has_no_adjustment_id() {
        let line = CycleCountApprovedLine {
            line_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            expected_qty: 100,
            counted_qty: 100,
            variance_qty: 0,
            adjustment_id: None,
        };
        assert_eq!(line.variance_qty, 0);
        assert!(line.adjustment_id.is_none());
    }

    #[test]
    fn payload_serializes_correctly() {
        let payload = make_payload();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("task_id"));
        assert!(json.contains("adjustment_count"));
        assert!(json.contains("adjustment_id"));
        assert!(json.contains("variance_qty"));
    }
}
