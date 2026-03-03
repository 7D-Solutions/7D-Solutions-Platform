//! Consumer contract tests for Phase 57 event types.
//!
//! These tests freeze the v1 event contracts by:
//! 1. Constructing real EventEnvelopes (no mocks)
//! 2. Serializing to JSON (real serde serialization)
//! 3. Validating envelope completeness per ADR-016
//! 4. Validating against JSON Schema files from contracts/events/
//! 5. Verifying stable event naming conventions
//!
//! Run: `cargo test -p platform_contracts`

use platform_contracts::{event_naming, mutation_classes, EventEnvelope};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

// ── Helpers ──────────────────────────────────────────────────────────

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("contracts")
}

fn load_schema(name: &str) -> serde_json::Value {
    let path = contracts_dir().join("events").join(name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read schema {}: {}", path.display(), e));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse schema {}: {}", name, e))
}

fn validate_against_schema(envelope_json: &serde_json::Value, schema_name: &str) {
    let schema_value = load_schema(schema_name);
    let compiled = jsonschema::JSONSchema::compile(&schema_value)
        .unwrap_or_else(|e| panic!("Failed to compile schema {}: {}", schema_name, e));

    let result = compiled.validate(envelope_json);
    if let Err(errors) = result {
        let error_msgs: Vec<String> = errors.map(|e| format!("  - {}", e)).collect();
        panic!(
            "Schema validation failed for {}:\n{}",
            schema_name,
            error_msgs.join("\n")
        );
    }
}

/// Validate all ADR-016 required fields are present and non-empty.
fn assert_envelope_completeness(json: &serde_json::Value, label: &str) {
    // Required fields (§1 of ADR-016)
    let required_fields = [
        "event_id",
        "event_type",
        "occurred_at",
        "tenant_id",
        "source_module",
        "source_version",
        "schema_version",
        "replay_safe",
        "mutation_class",
        "payload",
    ];

    for field in &required_fields {
        let val = json.get(field);
        assert!(
            val.is_some(),
            "[{}] Missing required field: {}",
            label, field
        );
        let val = val.unwrap();
        // String fields must be non-empty
        if let Some(s) = val.as_str() {
            assert!(!s.is_empty(), "[{}] Field '{}' is empty string", label, field);
        }
    }

    // event_id must be a valid UUID
    let event_id = json["event_id"].as_str().unwrap();
    assert!(
        Uuid::parse_str(event_id).is_ok(),
        "[{}] event_id '{}' is not a valid UUID",
        label,
        event_id
    );

    // tenant_id must be non-empty
    let tenant_id = json["tenant_id"].as_str().unwrap();
    assert!(
        !tenant_id.is_empty(),
        "[{}] tenant_id is empty",
        label
    );

    // mutation_class must be a known value
    let mc = json["mutation_class"].as_str().unwrap();
    assert!(
        mutation_classes::is_valid(mc),
        "[{}] Invalid mutation_class: '{}'",
        label,
        mc
    );

    // event_type must follow naming convention
    let et = json["event_type"].as_str().unwrap();
    // Event types in NATS subjects include the module prefix, but the event_type
    // field on the envelope may be the full subject or just the entity.action part.
    // We validate that it has at least entity.action structure.
    let type_part = if et.contains(".events.") {
        et.split(".events.").last().unwrap()
    } else {
        et
    };
    assert!(
        event_naming::validate_event_type(type_part).is_ok(),
        "[{}] event_type '{}' does not follow entity.action convention",
        label,
        et
    );

    // replay_safe must be a boolean
    assert!(
        json["replay_safe"].is_boolean(),
        "[{}] replay_safe must be a boolean",
        label
    );

    // payload must be an object
    assert!(
        json["payload"].is_object(),
        "[{}] payload must be an object",
        label
    );

    // source_version must look like semver
    let sv = json["source_version"].as_str().unwrap();
    assert!(
        sv.split('.').count() == 3,
        "[{}] source_version '{}' doesn't look like semver",
        label,
        sv
    );
}

fn build_envelope<T: Serialize>(
    tenant_id: &str,
    source_module: &str,
    event_type: &str,
    mutation_class: &str,
    payload: T,
) -> serde_json::Value {
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        source_module.to_string(),
        event_type.to_string(),
        payload,
    )
    .with_source_version("0.1.0".to_string())
    .with_schema_version("1".to_string())
    .with_mutation_class(Some(mutation_class.to_string()))
    .with_correlation_id(Some(Uuid::new_v4().to_string()))
    .with_causation_id(Some(Uuid::new_v4().to_string()))
    .with_replay_safe(true);

    serde_json::to_value(&envelope).expect("Failed to serialize envelope")
}

// ── Numbering Event Payloads ─────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct NumberAllocatedPayload {
    tenant_id: String,
    entity: String,
    number_value: i64,
    idempotency_key: String,
}

// ── Doc Mgmt Event Payloads ─────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct DocumentCreatedPayload {
    document_id: Uuid,
    doc_number: String,
    title: String,
    doc_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DocumentReleasedPayload {
    document_id: Uuid,
    doc_number: String,
    revision_number: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct RevisionCreatedPayload {
    document_id: Uuid,
    revision_id: Uuid,
    revision_number: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct DocumentDistributionRequestedPayload {
    distribution_id: Uuid,
    document_id: Uuid,
    revision_id: Option<Uuid>,
    doc_number: String,
    recipient_ref: String,
    channel: String,
    template_key: String,
    payload_json: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct DocumentDistributionStatusUpdatedPayload {
    distribution_id: Uuid,
    document_id: Uuid,
    status: String,
    provider_message_id: Option<String>,
    failure_reason: Option<String>,
}

// ── Workflow Event Payloads ──────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct InstanceStartedPayload {
    instance_id: Uuid,
    tenant_id: String,
    definition_id: Uuid,
    entity_type: String,
    entity_id: String,
    initial_step_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceAdvancedPayload {
    instance_id: Uuid,
    tenant_id: String,
    transition_id: Uuid,
    from_step_id: String,
    to_step_id: String,
    action: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceCompletedPayload {
    instance_id: Uuid,
    tenant_id: String,
    final_step_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceCancelledPayload {
    instance_id: Uuid,
    tenant_id: String,
    step_at_cancellation: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DefinitionCreatedPayload {
    definition_id: Uuid,
    tenant_id: String,
    name: String,
    version: i32,
    initial_step_id: String,
    step_count: usize,
}

// ── Identity Event Payloads ─────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct UserCreatedPayload {
    user_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct RoleAssignedPayload {
    user_id: Uuid,
    role_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct RoleRevokedPayload {
    user_id: Uuid,
    role_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccessReviewRecordedPayload {
    user_id: Uuid,
    reviewed_by: Uuid,
    review_id: Uuid,
    decision: String,
}

// ── Notifications Event Payloads ────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct DeliverySucceededPayload {
    notification_id: String,
    channel: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeliveryFailedPayload {
    notification_id: String,
    channel: String,
    status: String,
    failure_code: String,
}

// ══════════════════════════════════════════════════════════════════════
// NUMBERING
// ══════════════════════════════════════════════════════════════════════

#[test]
fn numbering_number_allocated_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "numbering",
        "number.allocated",
        mutation_classes::DATA_MUTATION,
        NumberAllocatedPayload {
            tenant_id: "tenant-001".into(),
            entity: "invoice".into(),
            number_value: 42,
            idempotency_key: "inv:create:tenant-001:req-1".into(),
        },
    );
    assert_envelope_completeness(&json, "numbering/number.allocated");
}

#[test]
fn numbering_number_allocated_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "numbering",
        "number.allocated",
        mutation_classes::DATA_MUTATION,
        NumberAllocatedPayload {
            tenant_id: "tenant-001".into(),
            entity: "work_order".into(),
            number_value: 1,
            idempotency_key: "wo:create:tenant-001:req-1".into(),
        },
    );
    validate_against_schema(&json, "numbering-number-allocated.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// DOC MGMT
// ══════════════════════════════════════════════════════════════════════

#[test]
fn doc_mgmt_document_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.created",
        mutation_classes::LIFECYCLE,
        DocumentCreatedPayload {
            document_id: Uuid::new_v4(),
            doc_number: "SOP-001".into(),
            title: "Standard Operating Procedure".into(),
            doc_type: "sop".into(),
        },
    );
    assert_envelope_completeness(&json, "doc_mgmt/document.created");
}

#[test]
fn doc_mgmt_document_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.created",
        mutation_classes::LIFECYCLE,
        DocumentCreatedPayload {
            document_id: Uuid::new_v4(),
            doc_number: "WI-042".into(),
            title: "Work Instruction #42".into(),
            doc_type: "work_instruction".into(),
        },
    );
    validate_against_schema(&json, "doc-mgmt-document-created.v1.json");
}

#[test]
fn doc_mgmt_document_released_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.released",
        mutation_classes::LIFECYCLE,
        DocumentReleasedPayload {
            document_id: Uuid::new_v4(),
            doc_number: "SOP-001".into(),
            revision_number: 3,
        },
    );
    assert_envelope_completeness(&json, "doc_mgmt/document.released");
}

#[test]
fn doc_mgmt_document_released_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.released",
        mutation_classes::LIFECYCLE,
        DocumentReleasedPayload {
            document_id: Uuid::new_v4(),
            doc_number: "SOP-001".into(),
            revision_number: 1,
        },
    );
    validate_against_schema(&json, "doc-mgmt-document-released.v1.json");
}

#[test]
fn doc_mgmt_revision_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "revision.created",
        mutation_classes::DATA_MUTATION,
        RevisionCreatedPayload {
            document_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
            revision_number: 2,
        },
    );
    assert_envelope_completeness(&json, "doc_mgmt/revision.created");
}

#[test]
fn doc_mgmt_revision_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "revision.created",
        mutation_classes::DATA_MUTATION,
        RevisionCreatedPayload {
            document_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
            revision_number: 5,
        },
    );
    validate_against_schema(&json, "doc-mgmt-revision-created.v1.json");
}

#[test]
fn doc_mgmt_distribution_requested_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.distribution.requested",
        mutation_classes::SIDE_EFFECT,
        DocumentDistributionRequestedPayload {
            distribution_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            revision_id: Some(Uuid::new_v4()),
            doc_number: "SOP-001".into(),
            recipient_ref: "qa@fireproof.test".into(),
            channel: "email".into(),
            template_key: "doc_distribution_notice".into(),
            payload_json: serde_json::json!({"priority":"high"}),
        },
    );
    assert_envelope_completeness(&json, "doc_mgmt/document.distribution.requested");
}

#[test]
fn doc_mgmt_distribution_requested_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.distribution.requested",
        mutation_classes::SIDE_EFFECT,
        DocumentDistributionRequestedPayload {
            distribution_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            revision_id: None,
            doc_number: "SOP-002".into(),
            recipient_ref: "ops@fireproof.test".into(),
            channel: "email".into(),
            template_key: "doc_distribution_notice".into(),
            payload_json: serde_json::json!({"doc":"release"}),
        },
    );
    validate_against_schema(&json, "doc-mgmt-document-distribution-requested.v1.json");
}

#[test]
fn doc_mgmt_distribution_status_updated_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.distribution.status.updated",
        mutation_classes::SIDE_EFFECT,
        DocumentDistributionStatusUpdatedPayload {
            distribution_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            status: "delivered".into(),
            provider_message_id: Some("provider-msg-42".into()),
            failure_reason: None,
        },
    );
    assert_envelope_completeness(&json, "doc_mgmt/document.distribution.status.updated");
}

#[test]
fn doc_mgmt_distribution_status_updated_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "doc_mgmt",
        "document.distribution.status.updated",
        mutation_classes::SIDE_EFFECT,
        DocumentDistributionStatusUpdatedPayload {
            distribution_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            status: "failed".into(),
            provider_message_id: None,
            failure_reason: Some("smtp_timeout".into()),
        },
    );
    validate_against_schema(
        &json,
        "doc-mgmt-document-distribution-status-updated.v1.json",
    );
}

// ══════════════════════════════════════════════════════════════════════
// WORKFLOW
// ══════════════════════════════════════════════════════════════════════

#[test]
fn workflow_instance_started_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.started",
        mutation_classes::DATA_MUTATION,
        InstanceStartedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            definition_id: Uuid::new_v4(),
            entity_type: "document".into(),
            entity_id: "doc-123".into(),
            initial_step_id: "draft_review".into(),
        },
    );
    assert_envelope_completeness(&json, "workflow/instance.started");
}

#[test]
fn workflow_instance_started_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.started",
        mutation_classes::DATA_MUTATION,
        InstanceStartedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            definition_id: Uuid::new_v4(),
            entity_type: "order".into(),
            entity_id: "ord-456".into(),
            initial_step_id: "pending_approval".into(),
        },
    );
    validate_against_schema(&json, "workflow-instance-started.v1.json");
}

#[test]
fn workflow_instance_advanced_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.advanced",
        mutation_classes::DATA_MUTATION,
        InstanceAdvancedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            transition_id: Uuid::new_v4(),
            from_step_id: "draft_review".into(),
            to_step_id: "approved".into(),
            action: "approve".into(),
        },
    );
    assert_envelope_completeness(&json, "workflow/instance.advanced");
}

#[test]
fn workflow_instance_advanced_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.advanced",
        mutation_classes::DATA_MUTATION,
        InstanceAdvancedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            transition_id: Uuid::new_v4(),
            from_step_id: "pending".into(),
            to_step_id: "in_progress".into(),
            action: "start_work".into(),
        },
    );
    validate_against_schema(&json, "workflow-instance-advanced.v1.json");
}

#[test]
fn workflow_instance_completed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.completed",
        mutation_classes::DATA_MUTATION,
        InstanceCompletedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            final_step_id: "final_review".into(),
        },
    );
    assert_envelope_completeness(&json, "workflow/instance.completed");
}

#[test]
fn workflow_instance_completed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.completed",
        mutation_classes::DATA_MUTATION,
        InstanceCompletedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            final_step_id: "shipped".into(),
        },
    );
    validate_against_schema(&json, "workflow-instance-completed.v1.json");
}

#[test]
fn workflow_instance_cancelled_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.cancelled",
        mutation_classes::DATA_MUTATION,
        InstanceCancelledPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            step_at_cancellation: "pending_approval".into(),
        },
    );
    assert_envelope_completeness(&json, "workflow/instance.cancelled");
}

#[test]
fn workflow_instance_cancelled_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "instance.cancelled",
        mutation_classes::DATA_MUTATION,
        InstanceCancelledPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            step_at_cancellation: "in_progress".into(),
        },
    );
    validate_against_schema(&json, "workflow-instance-cancelled.v1.json");
}

#[test]
fn workflow_definition_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "definition.created",
        mutation_classes::DATA_MUTATION,
        DefinitionCreatedPayload {
            definition_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            name: "Document Approval".into(),
            version: 1,
            initial_step_id: "draft".into(),
            step_count: 4,
        },
    );
    assert_envelope_completeness(&json, "workflow/definition.created");
}

#[test]
fn workflow_definition_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "definition.created",
        mutation_classes::DATA_MUTATION,
        DefinitionCreatedPayload {
            definition_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            name: "Order Fulfillment".into(),
            version: 2,
            initial_step_id: "received".into(),
            step_count: 6,
        },
    );
    validate_against_schema(&json, "workflow-definition-created.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// IDENTITY
// ══════════════════════════════════════════════════════════════════════

#[test]
fn identity_user_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.user_created",
        mutation_classes::LIFECYCLE,
        UserCreatedPayload {
            user_id: Uuid::new_v4(),
        },
    );
    assert_envelope_completeness(&json, "identity/user.lifecycle.user_created");
}

#[test]
fn identity_user_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.user_created",
        mutation_classes::LIFECYCLE,
        UserCreatedPayload {
            user_id: Uuid::new_v4(),
        },
    );
    validate_against_schema(&json, "identity-user-created.v1.json");
}

#[test]
fn identity_role_assigned_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.role_assigned",
        mutation_classes::LIFECYCLE,
        RoleAssignedPayload {
            user_id: Uuid::new_v4(),
            role_id: Uuid::new_v4(),
        },
    );
    assert_envelope_completeness(&json, "identity/user.lifecycle.role_assigned");
}

#[test]
fn identity_role_assigned_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.role_assigned",
        mutation_classes::LIFECYCLE,
        RoleAssignedPayload {
            user_id: Uuid::new_v4(),
            role_id: Uuid::new_v4(),
        },
    );
    validate_against_schema(&json, "identity-role-assigned.v1.json");
}

#[test]
fn identity_role_revoked_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.role_revoked",
        mutation_classes::LIFECYCLE,
        RoleRevokedPayload {
            user_id: Uuid::new_v4(),
            role_id: Uuid::new_v4(),
        },
    );
    assert_envelope_completeness(&json, "identity/user.lifecycle.role_revoked");
}

#[test]
fn identity_role_revoked_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.role_revoked",
        mutation_classes::LIFECYCLE,
        RoleRevokedPayload {
            user_id: Uuid::new_v4(),
            role_id: Uuid::new_v4(),
        },
    );
    validate_against_schema(&json, "identity-role-revoked.v1.json");
}

#[test]
fn identity_access_review_recorded_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.access_review_recorded",
        mutation_classes::LIFECYCLE,
        AccessReviewRecordedPayload {
            user_id: Uuid::new_v4(),
            reviewed_by: Uuid::new_v4(),
            review_id: Uuid::new_v4(),
            decision: "approved".into(),
        },
    );
    assert_envelope_completeness(
        &json,
        "identity/user.lifecycle.access_review_recorded",
    );
}

#[test]
fn identity_access_review_recorded_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "identity",
        "user.lifecycle.access_review_recorded",
        mutation_classes::LIFECYCLE,
        AccessReviewRecordedPayload {
            user_id: Uuid::new_v4(),
            reviewed_by: Uuid::new_v4(),
            review_id: Uuid::new_v4(),
            decision: "revoked".into(),
        },
    );
    validate_against_schema(&json, "identity-access-review-recorded.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// NOTIFICATIONS (existing schemas — verify Phase 57 compatibility)
// ══════════════════════════════════════════════════════════════════════

#[test]
fn notifications_delivery_succeeded_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "delivery.succeeded",
        mutation_classes::SIDE_EFFECT,
        DeliverySucceededPayload {
            notification_id: Uuid::new_v4().to_string(),
            channel: "email".into(),
            status: "succeeded".into(),
        },
    );
    assert_envelope_completeness(&json, "notifications/delivery.succeeded");
}

#[test]
fn notifications_delivery_failed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "delivery.failed",
        mutation_classes::SIDE_EFFECT,
        DeliveryFailedPayload {
            notification_id: Uuid::new_v4().to_string(),
            channel: "email".into(),
            status: "failed".into(),
            failure_code: "BOUNCED".into(),
        },
    );
    assert_envelope_completeness(&json, "notifications/delivery.failed");
}

// ══════════════════════════════════════════════════════════════════════
// CROSS-CUTTING: Event Naming Convention Tests
// ══════════════════════════════════════════════════════════════════════

#[test]
fn all_phase57_event_types_follow_naming_convention() {
    let event_types = [
        // Numbering
        "number.allocated",
        // Doc Mgmt
        "document.created",
        "document.released",
        "revision.created",
        "document.distribution.requested",
        "document.distribution.status.updated",
        // Workflow
        "instance.started",
        "instance.advanced",
        "instance.completed",
        "instance.cancelled",
        "definition.created",
        // Identity
        "user.lifecycle.user_created",
        "user.lifecycle.role_assigned",
        "user.lifecycle.role_revoked",
        "user.lifecycle.access_review_recorded",
        // Notifications
        "delivery.succeeded",
        "delivery.failed",
    ];

    for et in &event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow naming convention",
            et
        );
    }
}

#[test]
fn all_phase57_nats_subjects_have_correct_prefix() {
    let module_events = [
        ("numbering", "number.allocated"),
        ("doc_mgmt", "document.created"),
        ("doc_mgmt", "document.released"),
        ("doc_mgmt", "revision.created"),
        ("doc_mgmt", "document.distribution.requested"),
        ("doc_mgmt", "document.distribution.status.updated"),
        ("workflow", "instance.started"),
        ("workflow", "instance.advanced"),
        ("workflow", "instance.completed"),
        ("workflow", "instance.cancelled"),
        ("workflow", "definition.created"),
        ("identity", "user.lifecycle.user_created"),
        ("identity", "user.lifecycle.role_assigned"),
        ("identity", "user.lifecycle.role_revoked"),
        ("identity", "user.lifecycle.access_review_recorded"),
        ("notifications", "delivery.succeeded"),
        ("notifications", "delivery.failed"),
    ];

    for (module, event_type) in &module_events {
        let subject = event_naming::nats_subject(module, event_type);
        let expected_prefix = format!("{}.events.", module);
        assert!(
            subject.starts_with(&expected_prefix),
            "NATS subject '{}' should start with '{}'",
            subject,
            expected_prefix
        );
        assert!(
            subject.ends_with(event_type),
            "NATS subject '{}' should end with '{}'",
            subject,
            event_type
        );
    }
}

// ══════════════════════════════════════════════════════════════════════
// CROSS-CUTTING: Schema Versioning Stability
// ══════════════════════════════════════════════════════════════════════

#[test]
fn all_phase57_schemas_exist_on_disk() {
    let schemas = [
        "numbering-number-allocated.v1.json",
        "doc-mgmt-document-created.v1.json",
        "doc-mgmt-document-released.v1.json",
        "doc-mgmt-revision-created.v1.json",
        "doc-mgmt-document-distribution-requested.v1.json",
        "doc-mgmt-document-distribution-status-updated.v1.json",
        "workflow-instance-started.v1.json",
        "workflow-instance-advanced.v1.json",
        "workflow-instance-completed.v1.json",
        "workflow-instance-cancelled.v1.json",
        "workflow-definition-created.v1.json",
        "identity-user-created.v1.json",
        "identity-role-assigned.v1.json",
        "identity-role-revoked.v1.json",
        "identity-access-review-recorded.v1.json",
        // Pre-existing Phase 57 schemas
        "notifications-delivery-succeeded.v1.json",
        "notifications-delivery-failed.v1.json",
    ];

    let events_dir = contracts_dir().join("events");
    for schema_name in &schemas {
        let path = events_dir.join(schema_name);
        assert!(
            path.exists(),
            "Schema file missing: {}",
            path.display()
        );

        // Verify it's valid JSON
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", schema_name, e));
        let _: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", schema_name, e));
    }
}

#[test]
fn all_phase57_schemas_have_schema_version_field() {
    let schemas = [
        "numbering-number-allocated.v1.json",
        "doc-mgmt-document-created.v1.json",
        "doc-mgmt-document-released.v1.json",
        "doc-mgmt-revision-created.v1.json",
        "doc-mgmt-document-distribution-requested.v1.json",
        "doc-mgmt-document-distribution-status-updated.v1.json",
        "workflow-instance-started.v1.json",
        "workflow-instance-advanced.v1.json",
        "workflow-instance-completed.v1.json",
        "workflow-instance-cancelled.v1.json",
        "workflow-definition-created.v1.json",
        "identity-user-created.v1.json",
        "identity-role-assigned.v1.json",
        "identity-role-revoked.v1.json",
        "identity-access-review-recorded.v1.json",
    ];

    for schema_name in &schemas {
        let schema = load_schema(schema_name);
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no 'required' array", schema_name));

        let required_strs: Vec<&str> = required
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        assert!(
            required_strs.contains(&"schema_version"),
            "{}: 'schema_version' must be in required fields",
            schema_name
        );
        assert!(
            required_strs.contains(&"mutation_class"),
            "{}: 'mutation_class' must be in required fields",
            schema_name
        );
        assert!(
            required_strs.contains(&"replay_safe"),
            "{}: 'replay_safe' must be in required fields",
            schema_name
        );
    }
}

// ══════════════════════════════════════════════════════════════════════
// CROSS-CUTTING: Financial Module Guard
// ══════════════════════════════════════════════════════════════════════

#[test]
fn non_financial_modules_do_not_require_merchant_context() {
    // Phase 57 modules are NOT financial modules, so they should pass
    // validation without merchant_context
    let non_financial = ["numbering", "doc_mgmt", "workflow", "identity", "notifications"];
    for module in &non_financial {
        assert!(
            !mutation_classes::FINANCIAL_MODULES.contains(module),
            "{} should NOT be in FINANCIAL_MODULES list",
            module
        );
    }
}

#[test]
fn financial_modules_require_merchant_context() {
    // Verify financial modules are correctly listed
    let expected = ["ar", "gl", "payments", "ap", "treasury", "billing", "ttp"];
    for module in &expected {
        assert!(
            mutation_classes::FINANCIAL_MODULES.contains(module),
            "{} should be in FINANCIAL_MODULES list",
            module
        );
    }
}
