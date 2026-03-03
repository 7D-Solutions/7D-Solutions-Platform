//! Workforce competence event contracts (schema_version = 1)

pub const WC_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

pub const EVENT_TYPE_ARTIFACT_REGISTERED: &str = "workforce_competence.artifact_registered";
pub const EVENT_TYPE_COMPETENCE_ASSIGNED: &str = "workforce_competence.competence_assigned";
pub const EVENT_TYPE_AUTHORITY_GRANTED: &str = "workforce_competence.acceptance_authority_granted";
pub const EVENT_TYPE_AUTHORITY_REVOKED: &str = "workforce_competence.acceptance_authority_revoked";

/// Create a workforce-competence-scoped EventEnvelope.
///
/// Sets `source_module = "workforce_competence"` and `replay_safe = true`.
pub fn create_wc_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> event_bus::EventEnvelope<T> {
    event_bus::EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "workforce_competence".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(correlation_id.clone()))
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(mutation_class))
    .with_replay_safe(true)
}
