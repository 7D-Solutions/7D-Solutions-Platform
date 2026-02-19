//! Event contract: external_ref.deleted

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::{INTEGRATIONS_EVENT_SCHEMA_VERSION, MUTATION_CLASS_LIFECYCLE};

pub const EVENT_TYPE_EXTERNAL_REF_DELETED: &str = "external_ref.deleted";

/// Payload for external_ref.deleted events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalRefDeletedPayload {
    pub ref_id: i64,
    pub app_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub system: String,
    pub external_id: String,
    pub deleted_at: DateTime<Utc>,
}

pub fn build_external_ref_deleted_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ExternalRefDeletedPayload,
) -> EventEnvelope<ExternalRefDeletedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_EXTERNAL_REF_DELETED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(INTEGRATIONS_EVENT_SCHEMA_VERSION.to_string())
}
