//! outbound_webhook.created event — emitted when a tenant registers a new webhook.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::MUTATION_CLASS_DATA_MUTATION;

pub const EVENT_TYPE_OUTBOUND_WEBHOOK_CREATED: &str = "outbound_webhook.created";
pub const SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundWebhookCreatedPayload {
    pub webhook_id: Uuid,
    pub tenant_id: String,
    pub url: String,
    pub event_types: Vec<String>,
}

pub fn build_outbound_webhook_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OutboundWebhookCreatedPayload,
) -> EventEnvelope<OutboundWebhookCreatedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_OUTBOUND_WEBHOOK_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SCHEMA_VERSION.to_string())
    .with_replay_safe(true)
}
