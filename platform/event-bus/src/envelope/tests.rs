use super::*;
use serde_json::json;
use crate::envelope::validation::validate_merchant_context_for_financial;

#[test]
fn test_envelope_creation() {
    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "test-module".to_string(),
        "test.event".to_string(),
        json!({"test": "data"}),
    );

    assert_eq!(envelope.tenant_id, "tenant-123");
    assert_eq!(envelope.source_module, "test-module");
    assert_eq!(envelope.event_type, "test.event");
    assert_eq!(envelope.schema_version, "1.0.0");
    assert!(envelope.replay_safe);
    assert!(envelope.trace_id.is_none());
    assert!(envelope.correlation_id.is_none());
    assert!(envelope.causation_id.is_none());
    assert!(envelope.reverses_event_id.is_none());
    assert!(envelope.supersedes_event_id.is_none());
    assert!(envelope.side_effect_id.is_none());
    assert!(envelope.mutation_class.is_none());
    assert!(envelope.actor_id.is_none());
    assert!(envelope.actor_type.is_none());
}

#[test]
fn test_envelope_with_builder() {
    let reverses_id = uuid::Uuid::new_v4();
    let supersedes_id = uuid::Uuid::new_v4();

    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "test-module".to_string(),
        "test.event".to_string(),
        json!({"test": "data"}),
    )
    .with_source_version("1.2.3".to_string())
    .with_schema_version("2.0.0".to_string())
    .with_trace_id(Some("trace-123".to_string()))
    .with_correlation_id(Some("corr-456".to_string()))
    .with_causation_id(Some("cause-789".to_string()))
    .with_reverses_event_id(Some(reverses_id))
    .with_supersedes_event_id(Some(supersedes_id))
    .with_side_effect_id(Some("side-effect-123".to_string()))
    .with_replay_safe(false)
    .with_mutation_class(Some("financial".to_string()));

    assert_eq!(envelope.source_version, "1.2.3");
    assert_eq!(envelope.schema_version, "2.0.0");
    assert_eq!(envelope.trace_id, Some("trace-123".to_string()));
    assert_eq!(envelope.correlation_id, Some("corr-456".to_string()));
    assert_eq!(envelope.causation_id, Some("cause-789".to_string()));
    assert_eq!(envelope.reverses_event_id, Some(reverses_id));
    assert_eq!(envelope.supersedes_event_id, Some(supersedes_id));
    assert_eq!(envelope.side_effect_id, Some("side-effect-123".to_string()));
    assert!(!envelope.replay_safe);
    assert_eq!(envelope.mutation_class, Some("financial".to_string()));
}

#[test]
fn test_envelope_with_actor() {
    let actor_id = uuid::Uuid::new_v4();

    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "test-module".to_string(),
        "test.event".to_string(),
        json!({"test": "data"}),
    )
    .with_actor(actor_id, "User".to_string());

    assert_eq!(envelope.actor_id, Some(actor_id));
    assert_eq!(envelope.actor_type, Some("User".to_string()));
}

#[test]
fn test_validate_envelope_fields_valid() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {}
    });

    assert!(validate_envelope_fields(&envelope).is_ok());
}

#[test]
fn test_validate_envelope_fields_missing_tenant_id() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_empty_tenant_id() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_missing_event_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_missing_schema_version() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "replay_safe": true
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_missing_replay_safe() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0"
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_empty_event_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_empty_schema_version() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "",
        "replay_safe": true
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_empty_trace_id() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "trace_id": ""
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_empty_mutation_class() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": ""
    });

    assert!(validate_envelope_fields(&envelope).is_err());
}

#[test]
fn test_validate_envelope_fields_valid_with_optional_fields() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "trace_id": "trace-123",
        "correlation_id": "corr-456",
        "causation_id": "cause-789"
    });

    assert!(validate_envelope_fields(&envelope).is_ok());
}

#[test]
fn test_validate_envelope_fields_missing_mutation_class() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true
    });

    let result = validate_envelope_fields(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("mutation_class is required"));
}

#[test]
fn test_validate_envelope_fields_invalid_mutation_class() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "payment.succeeded",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "INVALID_CLASS"
    });

    let result = validate_envelope_fields(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid mutation_class"));
}

#[test]
fn test_validate_envelope_fields_valid_mutation_classes() {
    let valid_classes = vec![
        "DATA_MUTATION",
        "REVERSAL",
        "CORRECTION",
        "SIDE_EFFECT",
        "QUERY",
        "LIFECYCLE",
        "ADMINISTRATIVE",
    ];

    for class in valid_classes {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "test.event",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "test",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": class
        });

        assert!(
            validate_envelope_fields(&envelope).is_ok(),
            "Expected {} to be valid",
            class
        );
    }
}

#[test]
fn test_validate_envelope_fields_valid_actor_types() {
    let valid_actor_types = vec!["User", "Service", "System"];

    for actor_type in valid_actor_types {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "test.event",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "test",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "actor_id": "550e8400-e29b-41d4-a716-446655440001",
            "actor_type": actor_type
        });

        assert!(
            validate_envelope_fields(&envelope).is_ok(),
            "Expected {} to be valid",
            actor_type
        );
    }
}

#[test]
fn test_validate_envelope_fields_invalid_actor_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "test.event",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "test",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "actor_id": "550e8400-e29b-41d4-a716-446655440001",
        "actor_type": "InvalidType"
    });

    let result = validate_envelope_fields(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid actor_type"));
}

#[test]
fn test_validate_envelope_fields_empty_actor_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "test.event",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "test",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "actor_type": ""
    });

    let result = validate_envelope_fields(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("actor_type cannot be empty"));
}

// ── MerchantContext: struct tests ──────────────────────────────────────────

#[test]
fn test_merchant_context_serializes_tenant() {
    let ctx = MerchantContext::Tenant("tenant-abc".to_string());
    let json = serde_json::to_value(&ctx).unwrap();
    assert_eq!(json["type"], "Tenant");
    assert_eq!(json["id"], "tenant-abc");
}

#[test]
fn test_merchant_context_serializes_platform() {
    let ctx = MerchantContext::Platform;
    let json = serde_json::to_value(&ctx).unwrap();
    assert_eq!(json["type"], "Platform");
    assert!(json.get("id").is_none());
}

#[test]
fn test_merchant_context_deserializes_tenant() {
    let json = json!({"type": "Tenant", "id": "tenant-xyz"});
    let ctx: MerchantContext = serde_json::from_value(json).unwrap();
    assert_eq!(ctx, MerchantContext::Tenant("tenant-xyz".to_string()));
}

#[test]
fn test_merchant_context_deserializes_platform() {
    let json = json!({"type": "Platform"});
    let ctx: MerchantContext = serde_json::from_value(json).unwrap();
    assert_eq!(ctx, MerchantContext::Platform);
}

#[test]
fn test_envelope_carries_merchant_context() {
    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "ar".to_string(),
        "invoice.created".to_string(),
        json!({"invoice_id": "inv_001"}),
    )
    .with_merchant_context(Some(MerchantContext::Tenant("tenant-123".to_string())));

    assert_eq!(
        envelope.merchant_context,
        Some(MerchantContext::Tenant("tenant-123".to_string()))
    );

    // Verify round-trip serialization preserves the field
    let json = serde_json::to_value(&envelope).unwrap();
    let mc = &json["merchant_context"];
    assert_eq!(mc["type"], "Tenant");
    assert_eq!(mc["id"], "tenant-123");
}

#[test]
fn test_envelope_merchant_context_omitted_when_none() {
    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "notifications".to_string(),
        "notification.sent".to_string(),
        json!({}),
    );
    // merchant_context is None by default
    assert!(envelope.merchant_context.is_none());

    // When serialized, the field is omitted (skip_serializing_if = None)
    let json = serde_json::to_value(&envelope).unwrap();
    assert!(json.get("merchant_context").is_none());
}

// ── validate_merchant_context_for_financial: enforcement tests ─────────────

#[test]
fn test_financial_event_requires_merchant_context() {
    // AR module + DATA_MUTATION = financial event requiring merchant_context
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "invoice.created",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "ar",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {}
    });

    let result = validate_merchant_context_for_financial(&envelope);
    assert!(result.is_err(), "Financial event without merchant_context should be rejected");
    let err = result.unwrap_err();
    assert!(err.contains("merchant_context is required"), "Error was: {}", err);
}

#[test]
fn test_financial_event_accepts_tenant_merchant_context() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "invoice.created",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "ar",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "merchant_context": {"type": "Tenant", "id": "tenant-123"},
        "payload": {}
    });

    assert!(validate_merchant_context_for_financial(&envelope).is_ok());
}

#[test]
fn test_financial_event_accepts_platform_merchant_context() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "billing.charge_posted",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "platform",
        "source_module": "billing",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "merchant_context": {"type": "Platform"},
        "payload": {}
    });

    assert!(validate_merchant_context_for_financial(&envelope).is_ok());
}

#[test]
fn test_non_financial_module_skips_merchant_context_check() {
    // notifications is not a financial module — no merchant_context needed
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "notification.sent",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "notifications",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {}
    });

    assert!(validate_merchant_context_for_financial(&envelope).is_ok());
}

#[test]
fn test_non_financial_mutation_class_skips_merchant_context_check() {
    // ADMINISTRATIVE mutation class is not financial
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "config.updated",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "ar",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "ADMINISTRATIVE",
        "payload": {}
    });

    assert!(validate_merchant_context_for_financial(&envelope).is_ok());
}

#[test]
fn test_financial_event_rejects_empty_tenant_id_in_merchant_context() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "invoice.created",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "payments",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "merchant_context": {"type": "Tenant", "id": ""},
        "payload": {}
    });

    let result = validate_merchant_context_for_financial(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot be empty"));
}

#[test]
fn test_financial_event_rejects_invalid_merchant_context_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "invoice.created",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "gl",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "merchant_context": {"type": "Unknown"},
        "payload": {}
    });

    let result = validate_merchant_context_for_financial(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid merchant_context.type"));
}

#[test]
fn test_all_financial_modules_require_merchant_context() {
    let financial_modules = ["ar", "gl", "payments", "ap", "treasury", "billing", "ttp"];
    for module in financial_modules {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "test.event",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": module,
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "payload": {}
        });
        let result = validate_merchant_context_for_financial(&envelope);
        assert!(
            result.is_err(),
            "Module '{}' should require merchant_context but validation passed",
            module
        );
    }
}
