//! Event contract: inventory.item_change_recorded
//!
//! Emitted when a change history entry is recorded for an item revision
//! creation, activation, or policy update. This is the audit event that
//! proves governance evidence was captured.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_ITEM_CHANGE_RECORDED: &str = "inventory.item_change_recorded";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemChangeRecordedPayload {
    pub change_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub change_type: String,
    pub actor_id: String,
    pub diff: serde_json::Value,
    pub reason: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

pub fn build_item_change_recorded_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ItemChangeRecordedPayload,
) -> EventEnvelope<ItemChangeRecordedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ITEM_CHANGE_RECORDED.to_string(),
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
    fn item_change_recorded_envelope_has_correct_metadata() {
        let payload = ItemChangeRecordedPayload {
            change_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            revision_id: Some(Uuid::new_v4()),
            change_type: "revision_created".to_string(),
            actor_id: "user-123".to_string(),
            diff: serde_json::json!({"name": {"after": "Widget v2"}}),
            reason: Some("Spec update".to_string()),
            recorded_at: Utc::now(),
        };
        let envelope = build_item_change_recorded_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-change-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ITEM_CHANGE_RECORDED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
