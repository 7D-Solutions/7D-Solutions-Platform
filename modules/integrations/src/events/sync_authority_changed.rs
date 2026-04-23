//! Event contract: sync.authority.changed

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};

pub const EVENT_TYPE_SYNC_AUTHORITY_CHANGED: &str = "sync.authority.changed";

/// Payload for integrations.sync.authority.changed events.
/// Mirrors the JSON schema at contracts/events/integrations.sync.authority.changed.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncAuthorityChangedPayload {
    pub app_id: String,
    /// Connection row ID from integrations_oauth_connections.
    pub connector_id: Uuid,
    pub entity_type: String,
    /// Null when authority applies to all entities of this type.
    pub entity_id: Option<String>,
    pub previous_authority: String,
    pub new_authority: String,
    pub flipped_by: String,
}

pub fn build_sync_authority_changed_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: SyncAuthorityChangedPayload,
) -> EventEnvelope<SyncAuthorityChangedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_SYNC_AUTHORITY_CHANGED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn envelope_metadata_is_correct() {
        let payload = SyncAuthorityChangedPayload {
            app_id: "app-1".to_string(),
            connector_id: Uuid::new_v4(),
            entity_type: "invoice".to_string(),
            entity_id: None,
            previous_authority: "platform".to_string(),
            new_authority: "external".to_string(),
            flipped_by: "user-abc".to_string(),
        };
        let _ = Utc::now(); // ensure chrono compiled
        let env = build_sync_authority_changed_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_SYNC_AUTHORITY_CHANGED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
        assert_eq!(
            env.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
    }
}
