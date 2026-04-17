pub use event_bus::EventEnvelope;

use super::{CC_SCHEMA_VERSION, SOURCE_MODULE};

pub fn create_cc_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, SOURCE_MODULE.to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_schema_version(CC_SCHEMA_VERSION.to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
}
