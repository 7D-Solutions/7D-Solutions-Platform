//! Event contract: inventory.item_revision_activated
//!
//! Emitted when an item revision is activated for an effective window.
//! If a previously open revision was superseded, superseded_revision_id is set.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_ITEM_REVISION_ACTIVATED: &str = "inventory.item_revision_activated";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRevisionActivatedPayload {
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_number: i32,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    /// The revision that was superseded (its effective_to was set), if any.
    pub superseded_revision_id: Option<Uuid>,
    pub activated_at: DateTime<Utc>,
}

pub fn build_item_revision_activated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ItemRevisionActivatedPayload,
) -> EventEnvelope<ItemRevisionActivatedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ITEM_REVISION_ACTIVATED.to_string(),
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
    fn revision_activated_envelope_has_correct_metadata() {
        let payload = ItemRevisionActivatedPayload {
            revision_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            revision_number: 1,
            effective_from: Utc::now(),
            effective_to: None,
            superseded_revision_id: None,
            activated_at: Utc::now(),
        };
        let envelope = build_item_revision_activated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-rev-2".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ITEM_REVISION_ACTIVATED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
