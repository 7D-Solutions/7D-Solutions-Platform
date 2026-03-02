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
///
/// **Phase 40**: actor_id/actor_type carried from VerifiedClaims on HTTP mutations
pub fn create_ap_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    create_ap_envelope_with_actor(
        event_id,
        tenant_id,
        event_type,
        correlation_id,
        causation_id,
        mutation_class,
        payload,
        None,
        None,
    )
}

/// Helper to create an AP-specific envelope with actor identity.
///
/// Actor fields are propagated from the originating HTTP request's VerifiedClaims.
/// Pass `None` for both fields when the operation is system-initiated (consumers, schedulers).
pub fn create_ap_envelope_with_actor<T>(
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
    EventEnvelope::with_event_id(event_id, tenant_id, "ap".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
        .with_actor_from(actor_id, actor_type)
}
