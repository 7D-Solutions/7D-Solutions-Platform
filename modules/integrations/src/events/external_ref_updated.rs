//! Event contract: external_ref.updated

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

pub const EVENT_TYPE_EXTERNAL_REF_UPDATED: &str = "external_ref.updated";

/// Payload for external_ref.updated events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalRefUpdatedPayload {
    pub ref_id: i64,
    pub app_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub system: String,
    pub external_id: String,
    pub label: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub fn build_external_ref_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ExternalRefUpdatedPayload,
) -> EventEnvelope<ExternalRefUpdatedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_EXTERNAL_REF_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}
