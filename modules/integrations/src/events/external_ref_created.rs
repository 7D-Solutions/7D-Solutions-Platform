//! Event contract: external_ref.created

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_EXTERNAL_REF_CREATED: &str = "external_ref.created";

/// Payload for external_ref.created events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalRefCreatedPayload {
    pub ref_id: i64,
    pub app_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub system: String,
    pub external_id: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn build_external_ref_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ExternalRefCreatedPayload,
) -> EventEnvelope<ExternalRefCreatedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_EXTERNAL_REF_CREATED.to_string(),
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
    fn external_ref_created_envelope_metadata() {
        let payload = ExternalRefCreatedPayload {
            ref_id: 1,
            app_id: "app-1".to_string(),
            entity_type: "invoice".to_string(),
            entity_id: "inv-abc".to_string(),
            system: "stripe".to_string(),
            external_id: "in_123".to_string(),
            label: None,
            created_at: Utc::now(),
        };
        let env = build_external_ref_created_envelope(
            Uuid::new_v4(),
            "app-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(env.event_type, EVENT_TYPE_EXTERNAL_REF_CREATED);
        assert_eq!(env.source_module, "integrations");
        assert_eq!(env.schema_version, INTEGRATIONS_EVENT_SCHEMA_VERSION);
        assert!(env.replay_safe);
    }
}
