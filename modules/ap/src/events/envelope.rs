// Re-export the platform-wide event envelope
pub use event_bus::EventEnvelope;

/// Helper to create an AP-specific envelope.
///
/// All AP events carry:
/// - source_module: "ap"
/// - schema_version: "1.0.0" (callers override per-event)
/// - replay_safe: true (all AP events are self-contained at replay time)
/// - mutation_class: required (DATA_MUTATION, REVERSAL, or LIFECYCLE)
/// - trace_id: propagated from HTTP request context when available
pub fn create_ap_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, "ap".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
}
