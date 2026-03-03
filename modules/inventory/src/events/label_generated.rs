//! Event contract: inventory.label_generated.v1
//!
//! Emitted when a label is generated for an item. Downstream printing systems
//! can subscribe to this event to trigger physical label production.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_LABEL_GENERATED: &str = "inventory.label_generated.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelGeneratedPayload {
    pub label_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Uuid,
    pub revision_number: i32,
    pub label_type: String,
    pub barcode_format: String,
    pub payload: serde_json::Value,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

pub fn build_label_generated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: LabelGeneratedPayload,
) -> EventEnvelope<LabelGeneratedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_LABEL_GENERATED.to_string(),
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
    fn label_generated_envelope_has_correct_metadata() {
        let payload = LabelGeneratedPayload {
            label_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
            revision_number: 1,
            label_type: "item_label".to_string(),
            barcode_format: "code128".to_string(),
            payload: serde_json::json!({"barcode_value": "SKU-001-R1"}),
            actor_id: None,
            created_at: Utc::now(),
        };
        let envelope = build_label_generated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-lbl-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_LABEL_GENERATED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
