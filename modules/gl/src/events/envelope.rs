// Re-export the platform-wide event envelope
pub use event_bus::EventEnvelope;

/// Helper function to create a GL-specific envelope
///
/// Maps GL's domain-driven design concepts to the platform envelope:
/// - source_module is set to "gl" for all GL-originated events
/// - mutation_class is REQUIRED (see docs/governance/MUTATION-CLASSES.md)
///
/// **Phase 24b**: GL now emits its own events (accruals, reversals) in addition
/// to consuming them. This helper follows the same pattern as create_ar_envelope.
/// **Phase 34**: trace_id auto-populated from correlation_id for propagation
/// **Phase 40**: actor_id/actor_type carried from VerifiedClaims on HTTP mutations
pub fn create_gl_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    create_gl_envelope_with_actor(event_id, tenant_id, event_type, correlation_id, causation_id, mutation_class, payload, None, None)
}

/// Helper function to create a GL-specific envelope with actor identity.
///
/// Actor fields are propagated from the originating HTTP request's VerifiedClaims.
/// Pass `None` for both fields when the operation is system-initiated (NATS consumers, schedulers).
pub fn create_gl_envelope_with_actor<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
    actor_id: Option<uuid::Uuid>,
    actor_type: Option<String>,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, "gl".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
        .with_actor_from(actor_id, actor_type)
}
