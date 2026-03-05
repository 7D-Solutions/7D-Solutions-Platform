//! Event contract: inventory.make_buy_changed
//!
//! Emitted when an item's make/buy classification is set or changed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_MAKE_BUY_CHANGED: &str = "inventory.make_buy_changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakeBuyChangedPayload {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub previous_value: Option<String>,
    pub new_value: String,
    pub changed_at: DateTime<Utc>,
}

pub fn build_make_buy_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: MakeBuyChangedPayload,
) -> EventEnvelope<MakeBuyChangedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_MAKE_BUY_CHANGED.to_string(),
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
    fn make_buy_changed_envelope_has_correct_metadata() {
        let payload = MakeBuyChangedPayload {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            previous_value: None,
            new_value: "make".to_string(),
            changed_at: Utc::now(),
        };
        let envelope = build_make_buy_changed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-mb-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_MAKE_BUY_CHANGED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
