//! outbound_webhook.updated event — emitted when a webhook's URL, status, or subscriptions change.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::MUTATION_CLASS_DATA_MUTATION;

pub const EVENT_TYPE_OUTBOUND_WEBHOOK_UPDATED: &str = "outbound_webhook.updated";
pub const SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundWebhookUpdatedPayload {
    pub webhook_id: Uuid,
    pub tenant_id: String,
    pub url: String,
    pub status: String,
}

pub fn build_outbound_webhook_updated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OutboundWebhookUpdatedPayload,
) -> EventEnvelope<OutboundWebhookUpdatedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_OUTBOUND_WEBHOOK_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SCHEMA_VERSION.to_string())
    .with_replay_safe(true)
}
