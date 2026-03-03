//! Event contract: inventory.item_revision_created
//!
//! Emitted when a new item revision is created (draft state, not yet effective).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_ITEM_REVISION_CREATED: &str = "inventory.item_revision_created";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRevisionCreatedPayload {
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_number: i32,
    pub name: String,
    pub uom: String,
    pub change_reason: String,
    pub created_at: DateTime<Utc>,
}

pub fn build_item_revision_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ItemRevisionCreatedPayload,
) -> EventEnvelope<ItemRevisionCreatedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ITEM_REVISION_CREATED.to_string(),
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
    fn revision_created_envelope_has_correct_metadata() {
        let payload = ItemRevisionCreatedPayload {
            revision_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            revision_number: 1,
            name: "Widget v2".to_string(),
            uom: "ea".to_string(),
            change_reason: "Updated specifications".to_string(),
            created_at: Utc::now(),
        };
        let envelope = build_item_revision_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-rev-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ITEM_REVISION_CREATED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
