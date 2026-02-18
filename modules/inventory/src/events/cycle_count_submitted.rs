//! Event contract: inventory.cycle_count_submitted
//!
//! Emitted when a cycle count task is submitted (counted_qty filled in
//! for each line, variances computed). Stock changes are NOT applied yet —
//! that happens on approve (bd-opin).
//!
//! Idempotency: caller MUST supply a deterministic event_id derived from
//! the submit's stable business key (idempotency_key).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constant
// ============================================================================

/// A cycle count task was submitted (counted quantities recorded, variances computed)
pub const EVENT_TYPE_CYCLE_COUNT_SUBMITTED: &str = "inventory.cycle_count_submitted";

// ============================================================================
// Payload
// ============================================================================

/// One line within a submitted cycle count (per-item variance detail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleCountSubmittedLine {
    pub line_id: Uuid,
    pub item_id: Uuid,
    /// Quantity snapshotted at task creation time
    pub expected_qty: i64,
    /// Quantity physically counted by the operator
    pub counted_qty: i64,
    /// Computed: counted_qty − expected_qty (negative = shrinkage, positive = overage)
    pub variance_qty: i64,
}

/// Payload for inventory.cycle_count_submitted
///
/// Emitted when all (or selected) lines have been counted and the task
/// moves to 'submitted'. Stock is not yet adjusted — that happens on approve.
///
/// Idempotency: the outbox event_id is derived from the idempotency key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleCountSubmittedPayload {
    /// Stable task business key
    pub task_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub location_id: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub line_count: usize,
    pub lines: Vec<CycleCountSubmittedLine>,
}

// ============================================================================
// Envelope builder
// ============================================================================

/// Build an envelope for inventory.cycle_count_submitted
pub fn build_cycle_count_submitted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: CycleCountSubmittedPayload,
) -> EventEnvelope<CycleCountSubmittedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CYCLE_COUNT_SUBMITTED.to_string(),
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

    fn make_payload() -> CycleCountSubmittedPayload {
        CycleCountSubmittedPayload {
            task_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: Uuid::new_v4(),
            submitted_at: Utc::now(),
            line_count: 1,
            lines: vec![CycleCountSubmittedLine {
                line_id: Uuid::new_v4(),
                item_id: Uuid::new_v4(),
                expected_qty: 50,
                counted_qty: 45,
                variance_qty: -5,
            }],
        }
    }

    #[test]
    fn envelope_has_correct_metadata() {
        let payload = make_payload();
        let envelope = build_cycle_count_submitted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CYCLE_COUNT_SUBMITTED);
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
        let envelope = build_cycle_count_submitted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
    }

    #[test]
    fn payload_serializes_correctly() {
        let payload = make_payload();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("task_id"));
        assert!(json.contains("variance_qty"));
        assert!(json.contains("expected_qty"));
        assert!(json.contains("counted_qty"));
    }

    #[test]
    fn variance_qty_is_counted_minus_expected() {
        let line = CycleCountSubmittedLine {
            line_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            expected_qty: 100,
            counted_qty: 90,
            variance_qty: -10,
        };
        assert_eq!(line.variance_qty, line.counted_qty - line.expected_qty);
    }
}
