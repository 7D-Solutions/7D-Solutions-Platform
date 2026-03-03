//! Event contract: inventory.lot_merged.v1
//!
//! Emitted when multiple parent lots are merged into a single child lot.
//! Each parent edge carries the quantity contributed to the child.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::create_inventory_envelope;
use event_bus::EventEnvelope;

use super::{INVENTORY_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_LOT_MERGED: &str = "inventory.lot_merged.v1";

/// One parent lot contributing to the merge operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeParentEdge {
    pub parent_lot_id: Uuid,
    pub parent_lot_code: String,
    pub quantity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LotMergedPayload {
    pub operation_id: Uuid,
    pub tenant_id: String,
    pub child_lot_id: Uuid,
    pub child_lot_code: String,
    pub item_id: Uuid,
    pub parents: Vec<MergeParentEdge>,
    pub actor_id: Option<Uuid>,
    pub occurred_at: DateTime<Utc>,
}

pub fn build_lot_merged_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: LotMergedPayload,
) -> EventEnvelope<LotMergedPayload> {
    create_inventory_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_LOT_MERGED.to_string(),
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
    fn lot_merged_envelope_has_correct_metadata() {
        let payload = LotMergedPayload {
            operation_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            child_lot_id: Uuid::new_v4(),
            child_lot_code: "LOT-MERGED".to_string(),
            item_id: Uuid::new_v4(),
            parents: vec![MergeParentEdge {
                parent_lot_id: Uuid::new_v4(),
                parent_lot_code: "LOT-P1".to_string(),
                quantity: 10,
            }],
            actor_id: None,
            occurred_at: Utc::now(),
        };
        let envelope = build_lot_merged_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-merge-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_LOT_MERGED);
        assert_eq!(envelope.source_module, "inventory");
        assert!(envelope.replay_safe);
    }
}
