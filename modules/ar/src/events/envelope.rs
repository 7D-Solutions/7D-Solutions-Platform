// Re-export the platform-wide event envelope
pub use event_bus::EventEnvelope;

/// Helper function to create an AR-specific envelope
///
/// Maps AR's domain-driven design concepts to the platform envelope:
/// - trace_id: set to correlation_id when no explicit trace context provided
/// - aggregate fields are AR-internal and not exposed in the envelope
///
/// **Phase 16**: mutation_class is REQUIRED. Refer to docs/governance/MUTATION-CLASSES.md
/// **Phase 34**: trace_id auto-populated from correlation_id for propagation
pub fn create_ar_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, "ar".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
}
