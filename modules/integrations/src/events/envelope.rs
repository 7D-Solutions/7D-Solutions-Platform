//! Integrations event envelope builder.

pub use event_bus::EventEnvelope;

/// Create an integrations-specific event envelope.
///
/// All integrations events carry:
/// - source_module: "integrations"
/// - schema_version: "1.0.0"
/// - replay_safe: true (default)
/// - mutation_class: required
pub fn create_integrations_envelope<T>(
    event_id: uuid::Uuid,
    app_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(
        event_id,
        app_id,
        "integrations".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(correlation_id.clone()))
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(mutation_class))
}
