//! Event contract: inventory.status_changed
//!
//! Emitted when quantity is explicitly moved between status buckets
//! (available ↔ quarantine ↔ damaged). Each transfer is atomic and
//! idempotent; the event carries the full before/after bucket names.
//!
//! Idempotency: caller MUST supply a deterministic event_id derived from the
//! transfer's stable business key (idempotency_key).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constant
// ============================================================================

/// Stock quantity was moved between status buckets (available/quarantine/damaged)
pub const EVENT_TYPE_STATUS_CHANGED: &str = "inventory.status_changed";

// ============================================================================
// Payload
// ============================================================================

/// Payload for inventory.status_changed
///
/// Emitted when quantity is explicitly transferred between status buckets.
/// The `transfer_id` is the stable business key (UUID of the inv_status_transfers row).
///
/// Idempotency: the outbox event_id is derived from the idempotency key supplied
/// by the caller, so replays are safe across module restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusChangedPayload {
    /// Stable business key for this transfer (id of the inv_status_transfers row)
    pub transfer_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    /// Stock-keeping unit code (denormalized for consumer convenience)
    pub sku: String,
    pub warehouse_id: Uuid,
    /// Source bucket (available | quarantine | damaged)
    pub from_status: String,
    /// Destination bucket (available | quarantine | damaged)
    pub to_status: String,
    /// Quantity moved (always positive)
    pub quantity: i64,
    pub transferred_at: DateTime<Utc>,
}

// ============================================================================
// Envelope builder
// ============================================================================

/// Build an envelope for inventory.status_changed
pub fn build_status_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: StatusChangedPayload,
) -> EventEnvelope<StatusChangedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_STATUS_CHANGED.to_string(),
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

    fn make_payload() -> StatusChangedPayload {
        StatusChangedPayload {
            transfer_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            sku: "SKU-001".to_string(),
            warehouse_id: Uuid::new_v4(),
            from_status: "available".to_string(),
            to_status: "quarantine".to_string(),
            quantity: 10,
            transferred_at: Utc::now(),
        }
    }

    #[test]
    fn envelope_has_correct_metadata() {
        let payload = make_payload();
        let envelope = build_status_changed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_STATUS_CHANGED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, INVENTORY_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn payload_serializes_correctly() {
        let payload = make_payload();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("transfer_id"));
        assert!(json.contains("from_status"));
        assert!(json.contains("to_status"));
        assert!(json.contains("quantity"));
    }

    #[test]
    fn causation_id_propagated() {
        let payload = make_payload();
        let envelope = build_status_changed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
    }
}
