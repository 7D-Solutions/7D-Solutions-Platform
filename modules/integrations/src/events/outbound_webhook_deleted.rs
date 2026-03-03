//! outbound_webhook.deleted event — emitted when a webhook is soft-deleted.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};
use super::MUTATION_CLASS_LIFECYCLE;

pub const EVENT_TYPE_OUTBOUND_WEBHOOK_DELETED: &str = "outbound_webhook.deleted";
pub const SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundWebhookDeletedPayload {
    pub webhook_id: Uuid,
    pub tenant_id: String,
}

pub fn build_outbound_webhook_deleted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OutboundWebhookDeletedPayload,
) -> EventEnvelope<OutboundWebhookDeletedPayload> {
    create_integrations_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_OUTBOUND_WEBHOOK_DELETED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(SCHEMA_VERSION.to_string())
    .with_replay_safe(true)
}
