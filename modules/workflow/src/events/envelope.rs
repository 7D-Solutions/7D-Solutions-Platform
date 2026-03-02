//! EventEnvelope helpers for the workflow module.

use event_bus::outbox::validate_and_serialize_envelope;
pub use event_bus::{EventEnvelope, MerchantContext};
use serde::Serialize;
use uuid::Uuid;

pub fn create_envelope<T>(
    event_id: Uuid,
    tenant_id: String,
    event_type: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "workflow".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_mutation_class(Some("DATA_MUTATION".to_string()))
}

pub fn validate_envelope<T: Serialize>(
    envelope: &EventEnvelope<T>,
) -> Result<serde_json::Value, String> {
    validate_and_serialize_envelope(envelope)
}
