// Re-export the platform-wide event envelope
// This maintains backward compatibility for existing imports
pub use event_bus::EventEnvelope;

/// Helper function to create a payments-specific envelope
///
/// **Phase 16**: mutation_class is REQUIRED. Refer to docs/governance/MUTATION-CLASSES.md
/// **Phase 34**: trace_id auto-populated from correlation_id for propagation
/// **Phase 40**: actor_id/actor_type carried from VerifiedClaims on HTTP mutations
pub fn create_payments_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    create_payments_envelope_with_actor(event_id, tenant_id, event_type, correlation_id, causation_id, mutation_class, payload, None, None)
}

/// Helper function to create a payments-specific envelope with actor identity.
///
/// Actor fields are propagated from the originating HTTP request's VerifiedClaims.
/// Pass `None` for both fields when the operation is system-initiated (webhook processing, etc.).
pub fn create_payments_envelope_with_actor<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
    actor_id: Option<uuid::Uuid>,
    actor_type: Option<String>,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, tenant_id, "payments".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(correlation_id.clone())
        .with_correlation_id(correlation_id)
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
        .with_actor_from(actor_id, actor_type)
}
