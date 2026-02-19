// Re-export the platform-wide event envelope
pub use event_bus::{EventEnvelope, MerchantContext};

/// Helper function to create an AR-specific envelope
///
/// Maps AR's domain-driven design concepts to the platform envelope:
/// - trace_id: set to correlation_id when no explicit trace context provided
/// - aggregate fields are AR-internal and not exposed in the envelope
/// - actor_id/actor_type: propagated from the originating HTTP request claims
///
/// **Phase 16**: mutation_class is REQUIRED. Refer to docs/governance/MUTATION-CLASSES.md
/// **Phase 34**: trace_id auto-populated from correlation_id for propagation
/// **Phase 40**: actor_id/actor_type carried from VerifiedClaims on HTTP mutations
/// **Phase 40**: merchant_context auto-set to Tenant(tenant_id) — AR events are always
///               tenant-scoped (prevents accidental commingling with platform revenue).
pub fn create_ar_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    create_ar_envelope_with_actor(event_id, tenant_id, event_type, correlation_id, causation_id, mutation_class, payload, None, None)
}

/// Helper function to create an AR-specific envelope with actor identity.
///
/// Actor fields are propagated from the originating HTTP request's VerifiedClaims.
/// Pass `None` for both fields when the operation is system-initiated (schedulers, backfill).
///
/// merchant_context is always set to `Tenant(tenant_id)` — AR events are tenant-scoped
/// by design. Platform billing events must not use this helper.
pub fn create_ar_envelope_with_actor<T>(
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
    let merchant_ctx = MerchantContext::Tenant(tenant_id.clone());
    EventEnvelope::with_event_id(event_id, tenant_id, "ar".to_string(), event_type, payload)
        .with_source_version(env!("CARGO_PKG_VERSION").to_string())
        .with_trace_id(Some(correlation_id.clone()))
        .with_correlation_id(Some(correlation_id))
        .with_causation_id(causation_id)
        .with_mutation_class(Some(mutation_class))
        .with_actor_from(actor_id, actor_type)
        .with_merchant_context(Some(merchant_ctx))
}
