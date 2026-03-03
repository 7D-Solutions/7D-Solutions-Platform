//! Event contract: inventory.item_revision_policy_updated
//!
//! Emitted when policy flags on a draft item revision are updated.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED: &str = "inventory.item_revision_policy_updated";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRevisionPolicyUpdatedPayload {
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_number: i32,
    pub traceability_level: String,
    pub inspection_required: bool,
    pub shelf_life_days: Option<i32>,
    pub shelf_life_enforced: bool,
    pub updated_at: DateTime<Utc>,
}

pub fn build_item_revision_policy_updated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ItemRevisionPolicyUpdatedPayload,
) -> EventEnvelope<ItemRevisionPolicyUpdatedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED.to_string(),
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
    fn revision_policy_updated_envelope_has_correct_metadata() {
        let payload = ItemRevisionPolicyUpdatedPayload {
            revision_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            revision_number: 1,
            traceability_level: "serial".to_string(),
            inspection_required: true,
            shelf_life_days: Some(365),
            shelf_life_enforced: true,
            updated_at: Utc::now(),
        };
        let envelope = build_item_revision_policy_updated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-rev-pol-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
