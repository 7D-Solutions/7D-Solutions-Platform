//! webhook.routed event — emitted after an inbound webhook has been translated
//! to a domain event and written to the outbox.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_integrations_envelope, EventEnvelope};

pub const EVENT_TYPE_WEBHOOK_ROUTED: &str = "webhook.routed";
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Payload for `webhook.routed`.
///
/// Records what domain event was emitted as a result of routing the inbound
/// webhook. Useful for tracing and observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRoutedPayload {
    /// Foreign key to `integrations_webhook_ingest.id`.
    pub ingest_id: i64,
    /// Source system (e.g. "stripe").
    pub system: String,
    /// Original source event type (e.g. "invoice.payment_succeeded").
    pub source_event_type: Option<String>,
    /// Domain event type emitted (e.g. "payment.received").
    pub domain_event_type: String,
    /// Outbox event ID written.
    pub outbox_event_id: Uuid,
    /// When routing completed.
    pub routed_at: DateTime<Utc>,
}

pub fn build_webhook_routed_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: WebhookRoutedPayload,
) -> EventEnvelope<WebhookRoutedPayload> {
    create_integrations_envelope(
        event_id,
        app_id,
        EVENT_TYPE_WEBHOOK_ROUTED.to_string(),
        correlation_id,
        causation_id,
        "ROUTING".to_string(),
        payload,
    )
    .with_schema_version(SCHEMA_VERSION.to_string())
    .with_replay_safe(true)
}
