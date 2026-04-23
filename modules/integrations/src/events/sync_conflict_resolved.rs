//! Event contract: sync.conflict.resolved
//!
//! Emitted after a conflict row transitions to `resolved` and the DB commit
//! succeeds.  Uses the outbox pattern so the relay fires AFTER the commit —
//! the event never precedes the ledger transition.
//! Mirrors the JSON schema at contracts/events/integrations.sync.conflict.resolved.json.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_SYNC_CONFLICT_RESOLVED: &str = "sync.conflict.resolved";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictResolvedPayload {
    pub app_id: String,
    pub conflict_id: Uuid,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    /// "edit" | "creation" | "deletion"
    pub conflict_class: String,
    pub resolved_by: String,
    pub internal_id: String,
    pub resolution_note: Option<String>,
}

pub fn build_sync_conflict_resolved_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: SyncConflictResolvedPayload,
) -> EventEnvelope<SyncConflictResolvedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_SYNC_CONFLICT_RESOLVED.to_string(),
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
        let payload = SyncConflictResolvedPayload {
            app_id: "app-1".to_string(),
            conflict_id: Uuid::new_v4(),
            provider: "quickbooks".to_string(),
            entity_type: "invoice".to_string(),
            entity_id: "inv-42".to_string(),
            conflict_class: "edit".to_string(),
            resolved_by: "operator".to_string(),
            internal_id: "inv-internal-001".to_string(),
            resolution_note: Some("merged platform value".to_string()),
        };
        let env = build_sync_conflict_resolved_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_SYNC_CONFLICT_RESOLVED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert_eq!(
            env.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
    }

    #[test]
    fn all_entity_types_round_trip() {
        for entity_type in &["customer", "invoice", "payment"] {
            let payload = SyncConflictResolvedPayload {
                app_id: "app-2".to_string(),
                conflict_id: Uuid::nil(),
                provider: "quickbooks".to_string(),
                entity_type: entity_type.to_string(),
                entity_id: "ent-1".to_string(),
                conflict_class: "edit".to_string(),
                resolved_by: "op".to_string(),
                internal_id: "int-1".to_string(),
                resolution_note: None,
            };
            let json = serde_json::to_value(&payload).expect("serialize");
            assert_eq!(json["entity_type"], *entity_type);
            assert!(json["resolution_note"].is_null());
        }
    }
}
