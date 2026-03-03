//! Event contract: inventory.lot_split.v1
//!
//! Emitted when a lot is split into one or more child lots.
//! Each child edge carries the quantity transferred from the parent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_LOT_SPLIT: &str = "inventory.lot_split.v1";

/// One child lot created by the split operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitChildEdge {
    pub child_lot_id: Uuid,
    pub child_lot_code: String,
    pub quantity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LotSplitPayload {
    pub operation_id: Uuid,
    pub tenant_id: String,
    pub parent_lot_id: Uuid,
    pub parent_lot_code: String,
    pub item_id: Uuid,
    pub children: Vec<SplitChildEdge>,
    pub actor_id: Option<Uuid>,
    pub occurred_at: DateTime<Utc>,
}

pub fn build_lot_split_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: LotSplitPayload,
) -> EventEnvelope<LotSplitPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_LOT_SPLIT.to_string(),
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
    fn lot_split_envelope_has_correct_metadata() {
        let payload = LotSplitPayload {
            operation_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            parent_lot_id: Uuid::new_v4(),
            parent_lot_code: "LOT-PARENT".to_string(),
            item_id: Uuid::new_v4(),
            children: vec![SplitChildEdge {
                child_lot_id: Uuid::new_v4(),
                child_lot_code: "LOT-CHILD-1".to_string(),
                quantity: 5,
            }],
            actor_id: None,
            occurred_at: Utc::now(),
        };
        let envelope = build_lot_split_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-split-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_LOT_SPLIT);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
