//! Event contract: sync.conflict.detected
//!
//! Emitted when the sync detector identifies true external drift and opens a conflict row.
//! Mirrors the JSON schema at contracts/events/integrations.sync.conflict.detected.json.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_SYNC_CONFLICT_DETECTED: &str = "sync.conflict.detected";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictDetectedPayload {
    pub app_id: String,
    pub conflict_id: Uuid,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    /// "edit" | "creation" | "deletion"
    pub conflict_class: String,
    pub detected_by: String,
}

pub fn build_sync_conflict_detected_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: SyncConflictDetectedPayload,
) -> EventEnvelope<SyncConflictDetectedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_SYNC_CONFLICT_DETECTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_metadata_is_correct() {
        let payload = SyncConflictDetectedPayload {
            app_id: "app-1".to_string(),
            conflict_id: Uuid::new_v4(),
            provider: "quickbooks".to_string(),
            entity_type: "invoice".to_string(),
            entity_id: "inv-42".to_string(),
            conflict_class: "edit".to_string(),
            detected_by: "detector".to_string(),
        };
        let env = build_sync_conflict_detected_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_SYNC_CONFLICT_DETECTED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert_eq!(
            env.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
    }

    #[test]
    fn all_conflict_classes_round_trip() {
        for class in &["edit", "creation", "deletion"] {
            let payload = SyncConflictDetectedPayload {
                app_id: "app-2".to_string(),
                conflict_id: Uuid::nil(),
                provider: "quickbooks".to_string(),
                entity_type: "customer".to_string(),
                entity_id: "cust-1".to_string(),
                conflict_class: class.to_string(),
                detected_by: "detector".to_string(),
            };
            let json = serde_json::to_value(&payload).expect("serialize");
            assert_eq!(json["conflict_class"], *class);
        }
    }
}
