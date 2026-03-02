//! EventEnvelope helpers for the maintenance module.
//!
//! Wraps event payloads in a platform-compliant EventEnvelope with
//! maintenance-specific defaults (source_module, source_version, mutation_class).

use event_bus::outbox::validate_and_serialize_envelope;
pub use event_bus::{EventEnvelope, MerchantContext};
use serde::Serialize;
use uuid::Uuid;

/// Create a maintenance-module EventEnvelope.
///
/// Sets source_module = "maintenance", source_version from Cargo.toml,
/// and mutation_class = "DATA_MUTATION". Caller may further customise
/// the envelope via builder methods before enqueueing.
pub fn create_envelope<T>(
    event_id: Uuid,
    tenant_id: String,
    event_type: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "maintenance".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_mutation_class(Some("DATA_MUTATION".to_string()))
}

/// Validate and serialize an envelope for outbox storage.
///
/// Returns the full envelope as `serde_json::Value`, ready to be stored
/// in the outbox `payload` column.
pub fn validate_envelope<T: Serialize>(
    envelope: &EventEnvelope<T>,
) -> Result<serde_json::Value, String> {
    validate_and_serialize_envelope(envelope)
}
