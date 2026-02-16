// Re-export the platform-wide event envelope
// This maintains backward compatibility for existing imports
pub use event_bus::EventEnvelope;

// Helper function to create a payments-specific envelope
pub fn create_payments_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, "payments".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_correlation_id(correlation_id)
        .with_causation_id(causation_id)
}
