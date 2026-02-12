// Re-export the platform-wide event envelope
pub use event_bus::EventEnvelope;

/// Helper function to create a subscriptions-specific envelope
pub fn create_subscriptions_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, "subscriptions".to_string(), payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_correlation_id(correlation_id)
        .with_causation_id(causation_id)
}
