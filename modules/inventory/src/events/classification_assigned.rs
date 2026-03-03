//! Event contract: inventory.classification_assigned.v1
//!
//! Emitted when a classification or commodity code is assigned to an item.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_CLASSIFICATION_ASSIGNED: &str = "inventory.classification_assigned.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationAssignedPayload {
    pub classification_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub classification_system: String,
    pub classification_code: String,
    pub classification_label: Option<String>,
    pub commodity_system: Option<String>,
    pub commodity_code: Option<String>,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
}

pub fn build_classification_assigned_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ClassificationAssignedPayload,
) -> EventEnvelope<ClassificationAssignedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CLASSIFICATION_ASSIGNED.to_string(),
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
    fn classification_assigned_envelope_has_correct_metadata() {
        let payload = ClassificationAssignedPayload {
            classification_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            revision_id: None,
            classification_system: "UNSPSC".to_string(),
            classification_code: "31162800".to_string(),
            classification_label: Some("Fasteners".to_string()),
            commodity_system: Some("UNSPSC".to_string()),
            commodity_code: Some("31162800".to_string()),
            assigned_by: "user-1".to_string(),
            assigned_at: Utc::now(),
        };
        let envelope = build_classification_assigned_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-cls-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CLASSIFICATION_ASSIGNED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
