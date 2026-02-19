//! Party event envelope builder.

pub use event_bus::EventEnvelope;

/// Create a party-specific event envelope.
///
/// All party events carry:
/// - source_module: "party"
/// - schema_version: "1.0.0"
/// - replay_safe: true
/// - mutation_class: required
/// **Phase 40**: actor_id/actor_type carried from VerifiedClaims on HTTP mutations
pub fn create_party_envelope<T>(
    event_id: uuid::Uuid,
    app_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    create_party_envelope_with_actor(event_id, app_id, event_type, correlation_id, causation_id, mutation_class, payload, None, None)
}

/// Create a party-specific event envelope with actor identity.
///
/// Actor fields are propagated from the originating HTTP request's VerifiedClaims.
/// Pass `None` for both fields when the operation is system-initiated.
pub fn create_party_envelope_with_actor<T>(
    event_id: uuid::Uuid,
    app_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
    actor_id: Option<uuid::Uuid>,
    actor_type: Option<String>,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(event_id, app_id, "party".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
        .with_actor_from(actor_id, actor_type)
}
