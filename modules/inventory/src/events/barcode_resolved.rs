//! Event contract: inventory.barcode_resolved
//!
//! Emitted whenever a barcode is evaluated against tenant rules (resolved or not).
//! Observability event — downstream systems may subscribe to track scan activity.
//! No v1 suffix in the event_type string; version lives only in the contract filename.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_BARCODE_RESOLVED: &str = "inventory.barcode_resolved";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarcodeResolvedPayload {
    pub barcode_raw: String,
    pub entity_type: Option<String>,
    pub resolved: bool,
    pub matched_rule_id: Option<Uuid>,
    pub resolved_by: Option<String>,
    pub resolved_at: DateTime<Utc>,
}

pub fn build_barcode_resolved_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: BarcodeResolvedPayload,
) -> EventEnvelope<BarcodeResolvedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_BARCODE_RESOLVED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INVENTORY_EVENT_SCHEMA_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barcode_resolved_envelope_has_correct_metadata() {
        let payload = BarcodeResolvedPayload {
            barcode_raw: "LOT-TEST-001".to_string(),
            entity_type: Some("lot".to_string()),
            resolved: true,
            matched_rule_id: Some(Uuid::new_v4()),
            resolved_by: Some("user-1".to_string()),
            resolved_at: Utc::now(),
        };
        let envelope = build_barcode_resolved_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-bc-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_BARCODE_RESOLVED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn event_type_has_no_v1_suffix() {
        assert!(!EVENT_TYPE_BARCODE_RESOLVED.ends_with(".v1"));
    }
}
