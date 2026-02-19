//! webhook.received event — emitted after raw payload is persisted to ingest table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};

pub const EVENT_TYPE_WEBHOOK_RECEIVED: &str = "webhook.received";
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Payload for `webhook.received`.
///
/// Carries the ingest record ID so downstream consumers can load the raw
/// payload without duplicating it in the event bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookReceivedPayload {
    /// Primary key of `integrations_webhook_ingest`.
    pub ingest_id: i64,
    /// Source system name (e.g. "stripe", "github").
    pub system: String,
    /// Source event type if known (e.g. "invoice.payment_succeeded").
    pub event_type: Option<String>,
    /// Idempotency key supplied by source system.
    pub idempotency_key: Option<String>,
    /// When the payload was received.
    pub received_at: DateTime<Utc>,
}

pub fn build_webhook_received_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: WebhookReceivedPayload,
) -> EventEnvelope<WebhookReceivedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_WEBHOOK_RECEIVED.to_string(),
        correlation_id,
        causation_id,
        "INGEST".to_string(),
        payload,
    )
    .with_schema_version(SCHEMA_VERSION.to_string())
    .with_replay_safe(false) // Raw ingest — do not replay blindly
}
