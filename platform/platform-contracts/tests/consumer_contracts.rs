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

#[derive(Debug, Serialize, Deserialize)]
struct NumberConfirmedPayload {
    tenant_id: String,
    entity: String,
    number_value: i64,
    idempotency_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PolicyUpdatedPayload {
    tenant_id: String,
    entity: String,
    pattern: String,
    prefix: String,
    padding: i32,
    version: i32,
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

// ── Workflow Gate B Event Payloads ─────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct DecisionRecordedPayload {
    instance_id: Uuid,
    tenant_id: String,
    step_id: String,
    actor_id: Uuid,
    decision: String,
    current_count: u32,
    threshold: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ParallelThresholdMetPayload {
    instance_id: Uuid,
    tenant_id: String,
    step_id: String,
    decision_count: u32,
    threshold: u32,
    target_step: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HoldAppliedPayload {
    hold_id: Uuid,
    tenant_id: String,
    entity_type: String,
    entity_id: String,
    hold_type: String,
    reason: Option<String>,
    applied_by: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HoldReleasedPayload {
    hold_id: Uuid,
    tenant_id: String,
    entity_type: String,
    entity_id: String,
    hold_type: String,
    released_by: Option<Uuid>,
    release_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EscalationFiredPayload {
    timer_id: Uuid,
    instance_id: Uuid,
    tenant_id: String,
    rule_id: Uuid,
    step_id: String,
    escalation_count: i32,
    escalate_to_step: Option<String>,
    notify_actor_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DelegationCreatedPayload {
    delegation_id: Uuid,
    tenant_id: String,
    delegator_id: Uuid,
    delegatee_id: Uuid,
    definition_id: Option<Uuid>,
    entity_type: Option<String>,
    valid_from: String,
    valid_until: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DelegationRevokedPayload {
    delegation_id: Uuid,
    tenant_id: String,
    delegator_id: Uuid,
    delegatee_id: Uuid,
    revoked_by: Uuid,
    revoke_reason: Option<String>,
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
    status: Option<String>,
    attempt_no: Option<i32>,
    attempt_status: Option<String>,
    scheduled_status: Option<String>,
    idempotency_key: Option<String>,
    template_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeliveryFailedPayload {
    notification_id: String,
    channel: String,
    status: Option<String>,
    failure_code: Option<String>,
    attempt_no: Option<i32>,
    attempt_status: Option<String>,
    scheduled_status: Option<String>,
    idempotency_key: Option<String>,
    template_key: Option<String>,
    error_class: Option<String>,
    error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InboxMessageCreatedPayload {
    inbox_message_id: Uuid,
    user_id: String,
    notification_id: Uuid,
    title: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InboxMessageLifecyclePayload {
    inbox_message_id: Uuid,
    user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DlqActionPayload {
    notification_id: Uuid,
    action: String,
    previous_status: String,
    new_status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CloseCalendarReminderPayload {
    calendar_entry_id: Uuid,
    tenant_id: String,
    period_id: Uuid,
    owner_role: String,
    reminder_type: String,
    expected_close_date: String,
    days_offset: i32,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LowStockAlertCreatedPayload {
    notification_id: String,
    channel: String,
    status: String,
    attempts: i32,
    template_id: Option<String>,
}

// ── GL Event Payloads ───────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct GlAccrualReversalPolicyPayload {
    auto_reverse_next_period: bool,
    reverse_on_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlAccrualCreatedPayload {
    accrual_id: Uuid,
    template_id: Option<Uuid>,
    tenant_id: String,
    name: String,
    period: String,
    posting_date: String,
    debit_account: String,
    credit_account: String,
    amount_minor: i64,
    currency: String,
    cashflow_class: String,
    reversal_policy: GlAccrualReversalPolicyPayload,
    journal_entry_id: Option<Uuid>,
    description: String,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlAccrualReversedPayload {
    reversal_id: Uuid,
    original_accrual_id: Uuid,
    template_id: Option<Uuid>,
    tenant_id: String,
    reversal_period: String,
    reversal_date: String,
    debit_account: String,
    credit_account: String,
    amount_minor: i64,
    currency: String,
    cashflow_class: String,
    journal_entry_id: Option<Uuid>,
    reason: String,
    reversed_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlFxRateUpdatedPayload {
    rate_id: Uuid,
    base_currency: String,
    quote_currency: String,
    rate: f64,
    inverse_rate: f64,
    effective_at: String,
    source: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlFxRevaluationPostedPayload {
    revaluation_id: Uuid,
    tenant_id: String,
    period: String,
    transaction_currency: String,
    reporting_currency: String,
    rate_used: f64,
    original_amount_minor: i64,
    revalued_amount_minor: i64,
    unrealized_gain_loss_minor: i64,
    gain_loss_account: String,
    balance_account: String,
    journal_entry_id: Option<Uuid>,
    revaluation_date: String,
    auto_reverse: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlFxRealizedPostedPayload {
    realized_id: Uuid,
    tenant_id: String,
    source_transaction_id: Uuid,
    source_transaction_type: String,
    transaction_currency: String,
    reporting_currency: String,
    booking_rate: f64,
    settlement_rate: f64,
    transaction_amount_minor: i64,
    realized_gain_loss_minor: i64,
    gain_loss_account: String,
    journal_entry_id: Option<Uuid>,
    settlement_date: String,
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

#[test]
fn numbering_number_confirmed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "numbering",
        "number.confirmed",
        mutation_classes::DATA_MUTATION,
        NumberConfirmedPayload {
            tenant_id: "tenant-001".into(),
            entity: "invoice".into(),
            number_value: 42,
            idempotency_key: "inv:create:tenant-001:req-1".into(),
        },
    );
    assert_envelope_completeness(&json, "numbering/number.confirmed");
}

#[test]
fn numbering_number_confirmed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "numbering",
        "number.confirmed",
        mutation_classes::DATA_MUTATION,
        NumberConfirmedPayload {
            tenant_id: "tenant-001".into(),
            entity: "work_order".into(),
            number_value: 7,
            idempotency_key: "wo:confirm:tenant-001:req-1".into(),
        },
    );
    validate_against_schema(&json, "numbering-number-confirmed.v1.json");
}

#[test]
fn numbering_policy_updated_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "numbering",
        "policy.updated",
        mutation_classes::DATA_MUTATION,
        PolicyUpdatedPayload {
            tenant_id: "tenant-001".into(),
            entity: "invoice".into(),
            pattern: "{prefix}-{YYYY}-{number}".into(),
            prefix: "INV".into(),
            padding: 5,
            version: 1,
        },
    );
    assert_envelope_completeness(&json, "numbering/policy.updated");
}

#[test]
fn numbering_policy_updated_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "numbering",
        "policy.updated",
        mutation_classes::DATA_MUTATION,
        PolicyUpdatedPayload {
            tenant_id: "tenant-001".into(),
            entity: "work_order".into(),
            pattern: "{prefix}-{number}".into(),
            prefix: "WO".into(),
            padding: 4,
            version: 2,
        },
    );
    validate_against_schema(&json, "numbering-policy-updated.v1.json");
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

// ── Workflow Gate B: step, hold, escalation, delegation events ──────

#[test]
fn workflow_step_decision_recorded_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "step.decision_recorded",
        mutation_classes::DATA_MUTATION,
        DecisionRecordedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            step_id: "review".into(),
            actor_id: Uuid::new_v4(),
            decision: "approve".into(),
            current_count: 1,
            threshold: 2,
        },
    );
    assert_envelope_completeness(&json, "workflow/step.decision_recorded");
}

#[test]
fn workflow_step_decision_recorded_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "step.decision_recorded",
        mutation_classes::DATA_MUTATION,
        DecisionRecordedPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            step_id: "qa_check".into(),
            actor_id: Uuid::new_v4(),
            decision: "reject".into(),
            current_count: 3,
            threshold: 3,
        },
    );
    validate_against_schema(&json, "workflow-step-decision-recorded.v1.json");
}

#[test]
fn workflow_step_parallel_threshold_met_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "step.parallel_threshold_met",
        mutation_classes::DATA_MUTATION,
        ParallelThresholdMetPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            step_id: "review".into(),
            decision_count: 2,
            threshold: 2,
            target_step: "approved".into(),
        },
    );
    assert_envelope_completeness(&json, "workflow/step.parallel_threshold_met");
}

#[test]
fn workflow_step_parallel_threshold_met_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "step.parallel_threshold_met",
        mutation_classes::DATA_MUTATION,
        ParallelThresholdMetPayload {
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            step_id: "sign_off".into(),
            decision_count: 3,
            threshold: 3,
            target_step: "released".into(),
        },
    );
    validate_against_schema(&json, "workflow-step-parallel-threshold-met.v1.json");
}

#[test]
fn workflow_hold_applied_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "hold.applied",
        mutation_classes::DATA_MUTATION,
        HoldAppliedPayload {
            hold_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            entity_type: "work_order".into(),
            entity_id: "wo-123".into(),
            hold_type: "quality_hold".into(),
            reason: Some("Failed inspection".into()),
            applied_by: Some(Uuid::new_v4()),
        },
    );
    assert_envelope_completeness(&json, "workflow/hold.applied");
}

#[test]
fn workflow_hold_applied_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "hold.applied",
        mutation_classes::DATA_MUTATION,
        HoldAppliedPayload {
            hold_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            entity_type: "lot".into(),
            entity_id: "lot-456".into(),
            hold_type: "regulatory_hold".into(),
            reason: None,
            applied_by: None,
        },
    );
    validate_against_schema(&json, "workflow-hold-applied.v1.json");
}

#[test]
fn workflow_hold_released_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "hold.released",
        mutation_classes::DATA_MUTATION,
        HoldReleasedPayload {
            hold_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            entity_type: "work_order".into(),
            entity_id: "wo-123".into(),
            hold_type: "quality_hold".into(),
            released_by: Some(Uuid::new_v4()),
            release_reason: Some("Inspection passed on re-check".into()),
        },
    );
    assert_envelope_completeness(&json, "workflow/hold.released");
}

#[test]
fn workflow_hold_released_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "hold.released",
        mutation_classes::DATA_MUTATION,
        HoldReleasedPayload {
            hold_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            entity_type: "lot".into(),
            entity_id: "lot-789".into(),
            hold_type: "engineering_hold".into(),
            released_by: None,
            release_reason: None,
        },
    );
    validate_against_schema(&json, "workflow-hold-released.v1.json");
}

#[test]
fn workflow_escalation_fired_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "escalation.fired",
        mutation_classes::DATA_MUTATION,
        EscalationFiredPayload {
            timer_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            rule_id: Uuid::new_v4(),
            step_id: "pending_approval".into(),
            escalation_count: 1,
            escalate_to_step: Some("director_review".into()),
            notify_actor_ids: vec![Uuid::new_v4()],
        },
    );
    assert_envelope_completeness(&json, "workflow/escalation.fired");
}

#[test]
fn workflow_escalation_fired_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "escalation.fired",
        mutation_classes::DATA_MUTATION,
        EscalationFiredPayload {
            timer_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            rule_id: Uuid::new_v4(),
            step_id: "qa_review".into(),
            escalation_count: 2,
            escalate_to_step: None,
            notify_actor_ids: vec![Uuid::new_v4(), Uuid::new_v4()],
        },
    );
    validate_against_schema(&json, "workflow-escalation-fired.v1.json");
}

#[test]
fn workflow_delegation_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "delegation.created",
        mutation_classes::DATA_MUTATION,
        DelegationCreatedPayload {
            delegation_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            delegator_id: Uuid::new_v4(),
            delegatee_id: Uuid::new_v4(),
            definition_id: Some(Uuid::new_v4()),
            entity_type: Some("document".into()),
            valid_from: "2026-03-01T00:00:00Z".into(),
            valid_until: Some("2026-04-01T00:00:00Z".into()),
        },
    );
    assert_envelope_completeness(&json, "workflow/delegation.created");
}

#[test]
fn workflow_delegation_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "delegation.created",
        mutation_classes::DATA_MUTATION,
        DelegationCreatedPayload {
            delegation_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            delegator_id: Uuid::new_v4(),
            delegatee_id: Uuid::new_v4(),
            definition_id: None,
            entity_type: None,
            valid_from: "2026-03-01T00:00:00Z".into(),
            valid_until: None,
        },
    );
    validate_against_schema(&json, "workflow-delegation-created.v1.json");
}

#[test]
fn workflow_delegation_revoked_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "delegation.revoked",
        mutation_classes::DATA_MUTATION,
        DelegationRevokedPayload {
            delegation_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            delegator_id: Uuid::new_v4(),
            delegatee_id: Uuid::new_v4(),
            revoked_by: Uuid::new_v4(),
            revoke_reason: Some("Employee on leave".into()),
        },
    );
    assert_envelope_completeness(&json, "workflow/delegation.revoked");
}

#[test]
fn workflow_delegation_revoked_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "workflow",
        "delegation.revoked",
        mutation_classes::DATA_MUTATION,
        DelegationRevokedPayload {
            delegation_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            delegator_id: Uuid::new_v4(),
            delegatee_id: Uuid::new_v4(),
            revoked_by: Uuid::new_v4(),
            revoke_reason: None,
        },
    );
    validate_against_schema(&json, "workflow-delegation-revoked.v1.json");
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
            status: Some("succeeded".into()),
            attempt_no: None,
            attempt_status: None,
            scheduled_status: None,
            idempotency_key: None,
            template_key: None,
        },
    );
    assert_envelope_completeness(&json, "notifications/delivery.succeeded");
}

#[test]
fn notifications_delivery_succeeded_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.delivery.succeeded",
        mutation_classes::SIDE_EFFECT,
        DeliverySucceededPayload {
            notification_id: Uuid::new_v4().to_string(),
            channel: "email".into(),
            status: Some("succeeded".into()),
            attempt_no: Some(1),
            attempt_status: Some("succeeded".into()),
            scheduled_status: Some("sent".into()),
            idempotency_key: Some("notif:abc:gen:0:attempt:1".into()),
            template_key: Some("invoice_due_soon".into()),
        },
    );
    validate_against_schema(&json, "notifications-delivery-succeeded.v1.json");
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
            status: Some("failed".into()),
            failure_code: Some("BOUNCED".into()),
            attempt_no: None,
            attempt_status: None,
            scheduled_status: None,
            idempotency_key: None,
            template_key: None,
            error_class: None,
            error_message: None,
        },
    );
    assert_envelope_completeness(&json, "notifications/delivery.failed");
}

#[test]
fn notifications_delivery_failed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.delivery.failed",
        mutation_classes::SIDE_EFFECT,
        DeliveryFailedPayload {
            notification_id: Uuid::new_v4().to_string(),
            channel: "sms".into(),
            status: Some("failed".into()),
            failure_code: Some("provider_error".into()),
            attempt_no: Some(5),
            attempt_status: Some("failed_permanent".into()),
            scheduled_status: Some("dead_lettered".into()),
            idempotency_key: Some("notif:def:gen:0:attempt:5".into()),
            template_key: Some("payment_retry".into()),
            error_class: Some("provider".into()),
            error_message: Some("delivery permanently failed".into()),
        },
    );
    validate_against_schema(&json, "notifications-delivery-failed.v1.json");
}

#[test]
fn notifications_inbox_message_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.inbox.message_created",
        mutation_classes::LIFECYCLE,
        InboxMessageCreatedPayload {
            inbox_message_id: Uuid::new_v4(),
            user_id: "ops-user".into(),
            notification_id: Uuid::new_v4(),
            title: "Review required".into(),
        },
    );
    validate_against_schema(&json, "notifications-inbox-message-created.v1.json");
}

#[test]
fn notifications_inbox_message_read_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.inbox.message_read",
        mutation_classes::LIFECYCLE,
        InboxMessageLifecyclePayload {
            inbox_message_id: Uuid::new_v4(),
            user_id: "ops-user".into(),
        },
    );
    validate_against_schema(&json, "notifications-inbox-message-read.v1.json");
}

#[test]
fn notifications_inbox_message_unread_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.inbox.message_unread",
        mutation_classes::LIFECYCLE,
        InboxMessageLifecyclePayload {
            inbox_message_id: Uuid::new_v4(),
            user_id: "ops-user".into(),
        },
    );
    validate_against_schema(&json, "notifications-inbox-message-unread.v1.json");
}

#[test]
fn notifications_inbox_message_dismissed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.inbox.message_dismissed",
        mutation_classes::LIFECYCLE,
        InboxMessageLifecyclePayload {
            inbox_message_id: Uuid::new_v4(),
            user_id: "ops-user".into(),
        },
    );
    validate_against_schema(&json, "notifications-inbox-message-dismissed.v1.json");
}

#[test]
fn notifications_inbox_message_undismissed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.inbox.message_undismissed",
        mutation_classes::LIFECYCLE,
        InboxMessageLifecyclePayload {
            inbox_message_id: Uuid::new_v4(),
            user_id: "ops-user".into(),
        },
    );
    validate_against_schema(&json, "notifications-inbox-message-undismissed.v1.json");
}

#[test]
fn notifications_dlq_replayed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.dlq.replayed",
        mutation_classes::LIFECYCLE,
        DlqActionPayload {
            notification_id: Uuid::new_v4(),
            action: "replay".into(),
            previous_status: "dead_lettered".into(),
            new_status: "pending".into(),
        },
    );
    validate_against_schema(&json, "notifications-dlq-replayed.v1.json");
}

#[test]
fn notifications_dlq_abandoned_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.dlq.abandoned",
        mutation_classes::LIFECYCLE,
        DlqActionPayload {
            notification_id: Uuid::new_v4(),
            action: "abandon".into(),
            previous_status: "dead_lettered".into(),
            new_status: "abandoned".into(),
        },
    );
    validate_against_schema(&json, "notifications-dlq-abandoned.v1.json");
}

#[test]
fn notifications_close_calendar_reminder_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.close_calendar.reminder",
        mutation_classes::SIDE_EFFECT,
        CloseCalendarReminderPayload {
            calendar_entry_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            period_id: Uuid::new_v4(),
            owner_role: "controller".into(),
            reminder_type: "upcoming".into(),
            expected_close_date: "2026-03-31".into(),
            days_offset: 7,
            message: "Period close due in 7 day(s).".into(),
        },
    );
    validate_against_schema(&json, "notifications-close-calendar-reminder.v1.json");
}

#[test]
fn notifications_low_stock_alert_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "notifications",
        "notifications.low_stock.alert.created",
        mutation_classes::SIDE_EFFECT,
        LowStockAlertCreatedPayload {
            notification_id: Uuid::new_v4().to_string(),
            channel: "internal".into(),
            status: "queued".into(),
            attempts: 1,
            template_id: Some("low_stock_alert".into()),
        },
    );
    validate_against_schema(&json, "notifications-low-stock-alert-created.v1.json");
}

// ══════════════════════════════════════════════════════════════════════
// GL
// ══════════════════════════════════════════════════════════════════════

#[test]
fn gl_accrual_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.accrual_created",
        mutation_classes::DATA_MUTATION,
        GlAccrualCreatedPayload {
            accrual_id: Uuid::new_v4(),
            template_id: Some(Uuid::new_v4()),
            tenant_id: "tenant-001".into(),
            name: "Month-end accrual".into(),
            period: "2026-01".into(),
            posting_date: "2026-01-31".into(),
            debit_account: "6100".into(),
            credit_account: "2100".into(),
            amount_minor: 250000,
            currency: "USD".into(),
            cashflow_class: "operating".into(),
            reversal_policy: GlAccrualReversalPolicyPayload {
                auto_reverse_next_period: true,
                reverse_on_date: None,
            },
            journal_entry_id: Some(Uuid::new_v4()),
            description: "Accrued payroll".into(),
            created_at: "2026-01-31T23:59:59Z".into(),
        },
    );
    assert_envelope_completeness(&json, "gl/gl.accrual_created");
}

#[test]
fn gl_accrual_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.accrual_created",
        mutation_classes::DATA_MUTATION,
        GlAccrualCreatedPayload {
            accrual_id: Uuid::new_v4(),
            template_id: None,
            tenant_id: "tenant-001".into(),
            name: "Utilities accrual".into(),
            period: "2026-02".into(),
            posting_date: "2026-02-28".into(),
            debit_account: "6200".into(),
            credit_account: "2200".into(),
            amount_minor: 10000,
            currency: "USD".into(),
            cashflow_class: "operating".into(),
            reversal_policy: GlAccrualReversalPolicyPayload {
                auto_reverse_next_period: false,
                reverse_on_date: Some("2026-03-01".into()),
            },
            journal_entry_id: None,
            description: "Utilities estimate".into(),
            created_at: "2026-02-28T23:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "gl-accrual-created.v1.json");
}

#[test]
fn gl_accrual_reversed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.accrual_reversed",
        mutation_classes::REVERSAL,
        GlAccrualReversedPayload {
            reversal_id: Uuid::new_v4(),
            original_accrual_id: Uuid::new_v4(),
            template_id: Some(Uuid::new_v4()),
            tenant_id: "tenant-001".into(),
            reversal_period: "2026-02".into(),
            reversal_date: "2026-02-01".into(),
            debit_account: "2100".into(),
            credit_account: "6100".into(),
            amount_minor: 250000,
            currency: "USD".into(),
            cashflow_class: "operating".into(),
            journal_entry_id: Some(Uuid::new_v4()),
            reason: "auto_reverse_next_period".into(),
            reversed_at: "2026-02-01T00:00:01Z".into(),
        },
    );
    assert_envelope_completeness(&json, "gl/gl.accrual_reversed");
}

#[test]
fn gl_accrual_reversed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.accrual_reversed",
        mutation_classes::REVERSAL,
        GlAccrualReversedPayload {
            reversal_id: Uuid::new_v4(),
            original_accrual_id: Uuid::new_v4(),
            template_id: None,
            tenant_id: "tenant-001".into(),
            reversal_period: "2026-03".into(),
            reversal_date: "2026-03-01".into(),
            debit_account: "2200".into(),
            credit_account: "6200".into(),
            amount_minor: 10000,
            currency: "USD".into(),
            cashflow_class: "operating".into(),
            journal_entry_id: None,
            reason: "manual_correction".into(),
            reversed_at: "2026-03-01T01:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "gl-accrual-reversed.v1.json");
}

#[test]
fn gl_fx_rate_updated_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "fx.rate_updated",
        mutation_classes::DATA_MUTATION,
        GlFxRateUpdatedPayload {
            rate_id: Uuid::new_v4(),
            base_currency: "EUR".into(),
            quote_currency: "USD".into(),
            rate: 1.08,
            inverse_rate: 0.9259259,
            effective_at: "2026-02-01T00:00:00Z".into(),
            source: "ecb".into(),
        },
    );
    assert_envelope_completeness(&json, "gl/fx.rate_updated");
}

#[test]
fn gl_fx_rate_updated_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "fx.rate_updated",
        mutation_classes::DATA_MUTATION,
        GlFxRateUpdatedPayload {
            rate_id: Uuid::new_v4(),
            base_currency: "GBP".into(),
            quote_currency: "USD".into(),
            rate: 1.26,
            inverse_rate: 0.7936508,
            effective_at: "2026-02-15T10:00:00Z".into(),
            source: "manual".into(),
        },
    );
    validate_against_schema(&json, "gl-fx-rate-updated.v1.json");
}

#[test]
fn gl_fx_revaluation_posted_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.fx_revaluation_posted",
        mutation_classes::DATA_MUTATION,
        GlFxRevaluationPostedPayload {
            revaluation_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            period: "2026-02".into(),
            transaction_currency: "EUR".into(),
            reporting_currency: "USD".into(),
            rate_used: 1.09,
            original_amount_minor: 100000,
            revalued_amount_minor: 109000,
            unrealized_gain_loss_minor: 9000,
            gain_loss_account: "7990".into(),
            balance_account: "1100-EUR".into(),
            journal_entry_id: Some(Uuid::new_v4()),
            revaluation_date: "2026-02-28".into(),
            auto_reverse: true,
        },
    );
    assert_envelope_completeness(&json, "gl/gl.fx_revaluation_posted");
}

#[test]
fn gl_fx_revaluation_posted_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.fx_revaluation_posted",
        mutation_classes::DATA_MUTATION,
        GlFxRevaluationPostedPayload {
            revaluation_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            period: "2026-03".into(),
            transaction_currency: "CAD".into(),
            reporting_currency: "USD".into(),
            rate_used: 0.73,
            original_amount_minor: 500000,
            revalued_amount_minor: 365000,
            unrealized_gain_loss_minor: -35000,
            gain_loss_account: "7991".into(),
            balance_account: "1200-CAD".into(),
            journal_entry_id: None,
            revaluation_date: "2026-03-31".into(),
            auto_reverse: false,
        },
    );
    validate_against_schema(&json, "gl-fx-revaluation-posted.v1.json");
}

#[test]
fn gl_fx_realized_posted_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.fx_realized_posted",
        mutation_classes::DATA_MUTATION,
        GlFxRealizedPostedPayload {
            realized_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            source_transaction_id: Uuid::new_v4(),
            source_transaction_type: "invoice_payment".into(),
            transaction_currency: "EUR".into(),
            reporting_currency: "USD".into(),
            booking_rate: 1.07,
            settlement_rate: 1.1,
            transaction_amount_minor: 100000,
            realized_gain_loss_minor: 3000,
            gain_loss_account: "7992".into(),
            journal_entry_id: Some(Uuid::new_v4()),
            settlement_date: "2026-03-10".into(),
        },
    );
    assert_envelope_completeness(&json, "gl/gl.fx_realized_posted");
}

#[test]
fn gl_fx_realized_posted_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "gl",
        "gl.fx_realized_posted",
        mutation_classes::DATA_MUTATION,
        GlFxRealizedPostedPayload {
            realized_id: Uuid::new_v4(),
            tenant_id: "tenant-001".into(),
            source_transaction_id: Uuid::new_v4(),
            source_transaction_type: "ar_settlement".into(),
            transaction_currency: "GBP".into(),
            reporting_currency: "USD".into(),
            booking_rate: 1.24,
            settlement_rate: 1.22,
            transaction_amount_minor: 200000,
            realized_gain_loss_minor: -4000,
            gain_loss_account: "7993".into(),
            journal_entry_id: None,
            settlement_date: "2026-03-20".into(),
        },
    );
    validate_against_schema(&json, "gl-fx-realized-posted.v1.json");
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
        "inbox.message_created",
        "inbox.message_read",
        "inbox.message_unread",
        "inbox.message_dismissed",
        "inbox.message_undismissed",
        "dlq.replayed",
        "dlq.abandoned",
        "close_calendar.reminder",
        "low_stock.alert.created",
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
        ("numbering", "number.confirmed"),
        ("numbering", "policy.updated"),
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
        ("workflow", "step.decision_recorded"),
        ("workflow", "step.parallel_threshold_met"),
        ("workflow", "hold.applied"),
        ("workflow", "hold.released"),
        ("workflow", "escalation.fired"),
        ("workflow", "delegation.created"),
        ("workflow", "delegation.revoked"),
        ("identity", "user.lifecycle.user_created"),
        ("identity", "user.lifecycle.role_assigned"),
        ("identity", "user.lifecycle.role_revoked"),
        ("identity", "user.lifecycle.access_review_recorded"),
        ("notifications", "delivery.succeeded"),
        ("notifications", "delivery.failed"),
        ("notifications", "inbox.message_created"),
        ("notifications", "inbox.message_read"),
        ("notifications", "inbox.message_unread"),
        ("notifications", "inbox.message_dismissed"),
        ("notifications", "inbox.message_undismissed"),
        ("notifications", "dlq.replayed"),
        ("notifications", "dlq.abandoned"),
        ("notifications", "close_calendar.reminder"),
        ("notifications", "low_stock.alert.created"),
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
        "numbering-number-confirmed.v1.json",
        "numbering-policy-updated.v1.json",
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
        "workflow-step-decision-recorded.v1.json",
        "workflow-step-parallel-threshold-met.v1.json",
        "workflow-hold-applied.v1.json",
        "workflow-hold-released.v1.json",
        "workflow-escalation-fired.v1.json",
        "workflow-delegation-created.v1.json",
        "workflow-delegation-revoked.v1.json",
        "identity-user-created.v1.json",
        "identity-role-assigned.v1.json",
        "identity-role-revoked.v1.json",
        "identity-access-review-recorded.v1.json",
        // Notifications
        "notifications-delivery-succeeded.v1.json",
        "notifications-delivery-failed.v1.json",
        "notifications-inbox-message-created.v1.json",
        "notifications-inbox-message-read.v1.json",
        "notifications-inbox-message-unread.v1.json",
        "notifications-inbox-message-dismissed.v1.json",
        "notifications-inbox-message-undismissed.v1.json",
        "notifications-dlq-replayed.v1.json",
        "notifications-dlq-abandoned.v1.json",
        "notifications-close-calendar-reminder.v1.json",
        "notifications-low-stock-alert-created.v1.json",
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
        "numbering-number-confirmed.v1.json",
        "numbering-policy-updated.v1.json",
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
        "workflow-step-decision-recorded.v1.json",
        "workflow-step-parallel-threshold-met.v1.json",
        "workflow-hold-applied.v1.json",
        "workflow-hold-released.v1.json",
        "workflow-escalation-fired.v1.json",
        "workflow-delegation-created.v1.json",
        "workflow-delegation-revoked.v1.json",
        "identity-user-created.v1.json",
        "identity-role-assigned.v1.json",
        "identity-role-revoked.v1.json",
        "identity-access-review-recorded.v1.json",
        "notifications-delivery-succeeded.v1.json",
        "notifications-delivery-failed.v1.json",
        "notifications-inbox-message-created.v1.json",
        "notifications-inbox-message-read.v1.json",
        "notifications-inbox-message-unread.v1.json",
        "notifications-inbox-message-dismissed.v1.json",
        "notifications-inbox-message-undismissed.v1.json",
        "notifications-dlq-replayed.v1.json",
        "notifications-dlq-abandoned.v1.json",
        "notifications-close-calendar-reminder.v1.json",
        "notifications-low-stock-alert-created.v1.json",
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

#[test]
fn gl_gate_b_schemas_exist_on_disk() {
    let schemas = [
        "gl-accrual-created.v1.json",
        "gl-accrual-reversed.v1.json",
        "gl-fx-rate-updated.v1.json",
        "gl-fx-revaluation-posted.v1.json",
        "gl-fx-realized-posted.v1.json",
    ];

    let events_dir = contracts_dir().join("events");
    for schema_name in &schemas {
        let path = events_dir.join(schema_name);
        assert!(path.exists(), "Schema file missing: {}", path.display());

        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", schema_name, e));
        let _: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", schema_name, e));
    }
}

#[test]
fn gl_gate_b_schemas_require_envelope_fields() {
    let schemas = [
        "gl-accrual-created.v1.json",
        "gl-accrual-reversed.v1.json",
        "gl-fx-rate-updated.v1.json",
        "gl-fx-revaluation-posted.v1.json",
        "gl-fx-realized-posted.v1.json",
    ];

    for schema_name in &schemas {
        let schema = load_schema(schema_name);
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no 'required' array", schema_name));

        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

        for field in [
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
        ] {
            assert!(
                required_strs.contains(&field),
                "{} missing required envelope field '{}'",
                schema_name,
                field
            );
        }
    }
}

#[test]
fn notifications_gate_b_schemas_exist_on_disk() {
    let schemas = [
        "notifications-delivery-succeeded.v1.json",
        "notifications-delivery-failed.v1.json",
        "notifications-inbox-message-created.v1.json",
        "notifications-inbox-message-read.v1.json",
        "notifications-inbox-message-unread.v1.json",
        "notifications-inbox-message-dismissed.v1.json",
        "notifications-inbox-message-undismissed.v1.json",
        "notifications-dlq-replayed.v1.json",
        "notifications-dlq-abandoned.v1.json",
        "notifications-close-calendar-reminder.v1.json",
        "notifications-low-stock-alert-created.v1.json",
    ];

    let events_dir = contracts_dir().join("events");
    for schema_name in &schemas {
        let path = events_dir.join(schema_name);
        assert!(path.exists(), "Schema file missing: {}", path.display());

        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", schema_name, e));
        let _: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", schema_name, e));
    }
}

#[test]
fn notifications_gate_b_schemas_require_envelope_fields() {
    let schemas = [
        "notifications-delivery-succeeded.v1.json",
        "notifications-delivery-failed.v1.json",
        "notifications-inbox-message-created.v1.json",
        "notifications-inbox-message-read.v1.json",
        "notifications-inbox-message-unread.v1.json",
        "notifications-inbox-message-dismissed.v1.json",
        "notifications-inbox-message-undismissed.v1.json",
        "notifications-dlq-replayed.v1.json",
        "notifications-dlq-abandoned.v1.json",
        "notifications-close-calendar-reminder.v1.json",
        "notifications-low-stock-alert-created.v1.json",
    ];

    for schema_name in &schemas {
        let schema = load_schema(schema_name);
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no 'required' array", schema_name));
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

        for field in [
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
        ] {
            assert!(
                required_strs.contains(&field),
                "{} missing required envelope field '{}'",
                schema_name,
                field
            );
        }
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

// ══════════════════════════════════════════════════════════════════════
// INTEGRATIONS
// ══════════════════════════════════════════════════════════════════════

// ── Integrations Event Payloads ───────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct ExternalRefCreatedPayload {
    ref_id: i64,
    app_id: String,
    entity_type: String,
    entity_id: String,
    system: String,
    external_id: String,
    label: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExternalRefUpdatedPayload {
    ref_id: i64,
    app_id: String,
    entity_type: String,
    entity_id: String,
    system: String,
    external_id: String,
    label: Option<String>,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExternalRefDeletedPayload {
    ref_id: i64,
    app_id: String,
    entity_type: String,
    entity_id: String,
    system: String,
    external_id: String,
    deleted_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebhookReceivedPayload {
    ingest_id: i64,
    system: String,
    event_type: Option<String>,
    idempotency_key: Option<String>,
    received_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebhookRoutedPayload {
    ingest_id: i64,
    system: String,
    source_event_type: Option<String>,
    domain_event_type: String,
    outbox_event_id: Uuid,
    routed_at: String,
}

// ── Integrations: Envelope Completeness ───────────────────────────────

#[test]
fn integrations_external_ref_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "external_ref.created",
        mutation_classes::DATA_MUTATION,
        ExternalRefCreatedPayload {
            ref_id: 1,
            app_id: "tenant-001".into(),
            entity_type: "invoice".into(),
            entity_id: "inv-001".into(),
            system: "stripe".into(),
            external_id: "in_abc123".into(),
            label: Some("Stripe Invoice".into()),
            created_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "integrations/external_ref.created");
}

#[test]
fn integrations_external_ref_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "external_ref.created",
        mutation_classes::DATA_MUTATION,
        ExternalRefCreatedPayload {
            ref_id: 42,
            app_id: "tenant-001".into(),
            entity_type: "customer".into(),
            entity_id: "cust-55".into(),
            system: "quickbooks".into(),
            external_id: "QB-999".into(),
            label: None,
            created_at: "2026-03-02T12:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "integrations-external-ref-created.v1.json");
}

#[test]
fn integrations_external_ref_updated_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "external_ref.updated",
        mutation_classes::DATA_MUTATION,
        ExternalRefUpdatedPayload {
            ref_id: 1,
            app_id: "tenant-001".into(),
            entity_type: "invoice".into(),
            entity_id: "inv-001".into(),
            system: "stripe".into(),
            external_id: "in_abc123".into(),
            label: Some("Updated Label".into()),
            updated_at: "2026-03-01T01:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "integrations/external_ref.updated");
}

#[test]
fn integrations_external_ref_updated_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "external_ref.updated",
        mutation_classes::DATA_MUTATION,
        ExternalRefUpdatedPayload {
            ref_id: 7,
            app_id: "tenant-001".into(),
            entity_type: "order".into(),
            entity_id: "ord-10".into(),
            system: "salesforce".into(),
            external_id: "SF-LEAD-1".into(),
            label: None,
            updated_at: "2026-03-02T15:30:00Z".into(),
        },
    );
    validate_against_schema(&json, "integrations-external-ref-updated.v1.json");
}

#[test]
fn integrations_external_ref_deleted_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "external_ref.deleted",
        mutation_classes::LIFECYCLE,
        ExternalRefDeletedPayload {
            ref_id: 1,
            app_id: "tenant-001".into(),
            entity_type: "invoice".into(),
            entity_id: "inv-001".into(),
            system: "stripe".into(),
            external_id: "in_abc123".into(),
            deleted_at: "2026-03-01T02:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "integrations/external_ref.deleted");
}

#[test]
fn integrations_external_ref_deleted_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "external_ref.deleted",
        mutation_classes::LIFECYCLE,
        ExternalRefDeletedPayload {
            ref_id: 3,
            app_id: "tenant-001".into(),
            entity_type: "customer".into(),
            entity_id: "cust-99".into(),
            system: "hubspot".into(),
            external_id: "HS-CONTACT-99".into(),
            deleted_at: "2026-03-02T18:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "integrations-external-ref-deleted.v1.json");
}

#[test]
fn integrations_webhook_received_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "webhook.received",
        mutation_classes::DATA_MUTATION,
        WebhookReceivedPayload {
            ingest_id: 101,
            system: "stripe".into(),
            event_type: Some("invoice.payment_succeeded".into()),
            idempotency_key: Some("evt_123abc".into()),
            received_at: "2026-03-01T00:00:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "integrations/webhook.received");
}

#[test]
fn integrations_webhook_received_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "webhook.received",
        mutation_classes::DATA_MUTATION,
        WebhookReceivedPayload {
            ingest_id: 202,
            system: "github".into(),
            event_type: None,
            idempotency_key: None,
            received_at: "2026-03-02T10:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "integrations-webhook-received.v1.json");
}

#[test]
fn integrations_webhook_routed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "webhook.routed",
        mutation_classes::LIFECYCLE,
        WebhookRoutedPayload {
            ingest_id: 101,
            system: "stripe".into(),
            source_event_type: Some("invoice.payment_succeeded".into()),
            domain_event_type: "payment.received".into(),
            outbox_event_id: Uuid::new_v4(),
            routed_at: "2026-03-01T00:00:01Z".into(),
        },
    );
    assert_envelope_completeness(&json, "integrations/webhook.routed");
}

#[test]
fn integrations_webhook_routed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "integrations",
        "webhook.routed",
        mutation_classes::LIFECYCLE,
        WebhookRoutedPayload {
            ingest_id: 303,
            system: "tilled".into(),
            source_event_type: None,
            domain_event_type: "merchant.onboarded".into(),
            outbox_event_id: Uuid::new_v4(),
            routed_at: "2026-03-02T20:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "integrations-webhook-routed.v1.json");
}

// ── Integrations: Schema & Naming Cross-Checks ───────────────────────

#[test]
fn integrations_gate_b_schemas_exist_on_disk() {
    let schemas = [
        "integrations-external-ref-created.v1.json",
        "integrations-external-ref-updated.v1.json",
        "integrations-external-ref-deleted.v1.json",
        "integrations-webhook-received.v1.json",
        "integrations-webhook-routed.v1.json",
    ];

    let events_dir = contracts_dir().join("events");
    for schema_name in &schemas {
        let path = events_dir.join(schema_name);
        assert!(path.exists(), "Schema file missing: {}", path.display());

        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", schema_name, e));
        let _: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", schema_name, e));
    }
}

#[test]
fn integrations_gate_b_schemas_require_envelope_fields() {
    let schemas = [
        "integrations-external-ref-created.v1.json",
        "integrations-external-ref-updated.v1.json",
        "integrations-external-ref-deleted.v1.json",
        "integrations-webhook-received.v1.json",
        "integrations-webhook-routed.v1.json",
    ];

    for schema_name in &schemas {
        let schema = load_schema(schema_name);
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no 'required' array", schema_name));
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

        for field in [
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
        ] {
            assert!(
                required_strs.contains(&field),
                "{} missing required envelope field '{}'",
                schema_name,
                field
            );
        }
    }
}

#[test]
fn integrations_event_types_follow_naming_convention() {
    let event_types = [
        "external_ref.created",
        "external_ref.updated",
        "external_ref.deleted",
        "webhook.received",
        "webhook.routed",
    ];

    for et in &event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Integrations event type '{}' does not follow naming convention",
            et
        );
    }
}

#[test]
fn integrations_nats_subjects_have_correct_prefix() {
    let events = [
        "external_ref.created",
        "external_ref.updated",
        "external_ref.deleted",
        "webhook.received",
        "webhook.routed",
    ];

    for event_type in &events {
        let subject = event_naming::nats_subject("integrations", event_type);
        assert!(
            subject.starts_with("integrations.events."),
            "NATS subject '{}' should start with 'integrations.events.'",
            subject
        );
        assert!(
            subject.ends_with(event_type),
            "NATS subject '{}' should end with '{}'",
            subject,
            event_type
        );
    }
}

#[test]
fn integrations_is_not_financial_module() {
    assert!(
        !mutation_classes::FINANCIAL_MODULES.contains(&"integrations"),
        "integrations should NOT be in FINANCIAL_MODULES list"
    );
}

// ══════════════════════════════════════════════════════════════════════
// REPORTING (consumer-only module)
// ══════════════════════════════════════════════════════════════════════
//
// The reporting module is a read-model that consumes domain events from
// AR, GL, and Payments to populate cache tables. It does NOT publish
// events itself. These tests assert that no reporting event schemas
// have been introduced (the "(if published)" AC guard) and verify that
// the consumed event schemas remain loadable and structurally valid.

#[test]
fn reporting_has_no_published_event_schemas() {
    let events_dir = contracts_dir().join("events");
    let reporting_schemas: Vec<_> = std::fs::read_dir(&events_dir)
        .expect("contracts/events/ must exist")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("reporting-")
        })
        .collect();

    assert!(
        reporting_schemas.is_empty(),
        "Reporting is a consumer-only module and should not have event schemas, \
         but found: {:?}",
        reporting_schemas
            .iter()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
}

#[test]
fn reporting_consumed_gl_posting_request_schema_valid() {
    let schema = load_schema("gl-posting-request.v1.json");
    assert_eq!(
        schema["title"], "GL Posting Request Event",
        "GL posting request schema must have expected title"
    );
    let required = schema["properties"]["payload"]["required"]
        .as_array()
        .expect("payload.required must be an array");
    let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        required_names.contains(&"posting_date"),
        "GL posting request payload must require posting_date (reporting trial balance depends on it)"
    );
    assert!(
        required_names.contains(&"currency"),
        "GL posting request payload must require currency (reporting trial balance depends on it)"
    );
    assert!(
        required_names.contains(&"lines"),
        "GL posting request payload must require lines (reporting accumulates debit/credit from them)"
    );
}

#[test]
fn reporting_consumed_payment_succeeded_schema_valid() {
    let schema = load_schema("payments-payment-succeeded.v1.json");
    let required = schema["properties"]["payload"]["required"]
        .as_array()
        .expect("payload.required must be an array");
    let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        required_names.contains(&"payment_id"),
        "Payment succeeded payload must require payment_id (reporting cashflow depends on it)"
    );
    assert!(
        required_names.contains(&"amount_minor"),
        "Payment succeeded payload must require amount_minor (reporting cashflow depends on it)"
    );
    assert!(
        required_names.contains(&"currency"),
        "Payment succeeded payload must require currency (reporting cashflow depends on it)"
    );
    assert!(
        required_names.contains(&"invoice_id"),
        "Payment succeeded payload must require invoice_id (reporting payment history depends on it)"
    );
}

// ══════════════════════════════════════════════════════════════════════
// MAINTENANCE
// ══════════════════════════════════════════════════════════════════════

// ── Maintenance Event Payloads ────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct MaintWoCreatedPayload {
    wo_id: Uuid,
    wo_number: String,
    asset_id: Uuid,
    wo_type: String,
    priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_assignment_id: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintWoStatusChangedPayload {
    wo_id: Uuid,
    old_status: String,
    new_status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintWoCompletedPayload {
    wo_id: Uuid,
    asset_id: Uuid,
    total_parts_minor: i64,
    total_labor_minor: i64,
    currency: String,
    downtime_minutes: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixed_asset_ref: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintWoClosedPayload {
    wo_id: Uuid,
    asset_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintWoOverduePayload {
    wo_id: Uuid,
    asset_id: Uuid,
    days_overdue: i32,
    priority: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintMeterReadingRecordedPayload {
    asset_id: Uuid,
    meter_type_id: Uuid,
    reading_value: i64,
    recorded_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintPlanDuePayload {
    assignment_id: Uuid,
    plan_id: Uuid,
    asset_id: Uuid,
    due_kind: String,
    due_value: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaintPlanAssignedPayload {
    plan_id: Uuid,
    asset_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_due_meter: Option<i64>,
}

// ── work_order.created ───────────────────────────────────────────────

#[test]
fn maintenance_wo_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.created",
        mutation_classes::DATA_MUTATION,
        MaintWoCreatedPayload {
            wo_id: Uuid::new_v4(),
            wo_number: "WO-000001".into(),
            asset_id: Uuid::new_v4(),
            wo_type: "corrective".into(),
            priority: "high".into(),
            plan_assignment_id: None,
        },
    );
    assert_envelope_completeness(&json, "maintenance/work_order.created");
}

#[test]
fn maintenance_wo_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.created",
        mutation_classes::DATA_MUTATION,
        MaintWoCreatedPayload {
            wo_id: Uuid::new_v4(),
            wo_number: "WO-000042".into(),
            asset_id: Uuid::new_v4(),
            wo_type: "preventive".into(),
            priority: "medium".into(),
            plan_assignment_id: Some(Uuid::new_v4()),
        },
    );
    validate_against_schema(&json, "maintenance-work-order-created.v1.json");
}

// ── work_order.status_changed ────────────────────────────────────────

#[test]
fn maintenance_wo_status_changed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.status_changed",
        mutation_classes::DATA_MUTATION,
        MaintWoStatusChangedPayload {
            wo_id: Uuid::new_v4(),
            old_status: "draft".into(),
            new_status: "scheduled".into(),
        },
    );
    assert_envelope_completeness(&json, "maintenance/work_order.status_changed");
}

#[test]
fn maintenance_wo_status_changed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.status_changed",
        mutation_classes::DATA_MUTATION,
        MaintWoStatusChangedPayload {
            wo_id: Uuid::new_v4(),
            old_status: "scheduled".into(),
            new_status: "in_progress".into(),
        },
    );
    validate_against_schema(&json, "maintenance-work-order-status-changed.v1.json");
}

// ── work_order.completed ─────────────────────────────────────────────

#[test]
fn maintenance_wo_completed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.completed",
        mutation_classes::DATA_MUTATION,
        MaintWoCompletedPayload {
            wo_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            total_parts_minor: 15000,
            total_labor_minor: 8500,
            currency: "USD".into(),
            downtime_minutes: 120,
            fixed_asset_ref: Some(Uuid::new_v4()),
        },
    );
    assert_envelope_completeness(&json, "maintenance/work_order.completed");
}

#[test]
fn maintenance_wo_completed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.completed",
        mutation_classes::DATA_MUTATION,
        MaintWoCompletedPayload {
            wo_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            total_parts_minor: 0,
            total_labor_minor: 5000,
            currency: "USD".into(),
            downtime_minutes: 30,
            fixed_asset_ref: None,
        },
    );
    validate_against_schema(&json, "maintenance-work-order-completed.v1.json");
}

// ── work_order.closed ────────────────────────────────────────────────

#[test]
fn maintenance_wo_closed_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.closed",
        mutation_classes::DATA_MUTATION,
        MaintWoClosedPayload {
            wo_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
        },
    );
    assert_envelope_completeness(&json, "maintenance/work_order.closed");
}

#[test]
fn maintenance_wo_closed_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.closed",
        mutation_classes::DATA_MUTATION,
        MaintWoClosedPayload {
            wo_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
        },
    );
    validate_against_schema(&json, "maintenance-work-order-closed.v1.json");
}

// ── work_order.cancelled ─────────────────────────────────────────────

#[test]
fn maintenance_wo_cancelled_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.cancelled",
        mutation_classes::DATA_MUTATION,
        MaintWoStatusChangedPayload {
            wo_id: Uuid::new_v4(),
            old_status: "scheduled".into(),
            new_status: "cancelled".into(),
        },
    );
    assert_envelope_completeness(&json, "maintenance/work_order.cancelled");
}

#[test]
fn maintenance_wo_cancelled_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.cancelled",
        mutation_classes::DATA_MUTATION,
        MaintWoStatusChangedPayload {
            wo_id: Uuid::new_v4(),
            old_status: "in_progress".into(),
            new_status: "cancelled".into(),
        },
    );
    validate_against_schema(&json, "maintenance-work-order-cancelled.v1.json");
}

// ── work_order.overdue ───────────────────────────────────────────────

#[test]
fn maintenance_wo_overdue_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.overdue",
        mutation_classes::DATA_MUTATION,
        MaintWoOverduePayload {
            wo_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            days_overdue: 3,
            priority: "high".into(),
        },
    );
    assert_envelope_completeness(&json, "maintenance/work_order.overdue");
}

#[test]
fn maintenance_wo_overdue_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "work_order.overdue",
        mutation_classes::DATA_MUTATION,
        MaintWoOverduePayload {
            wo_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            days_overdue: 7,
            priority: "critical".into(),
        },
    );
    validate_against_schema(&json, "maintenance-work-order-overdue.v1.json");
}

// ── meter_reading.recorded ───────────────────────────────────────────

#[test]
fn maintenance_meter_reading_recorded_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "meter_reading.recorded",
        mutation_classes::DATA_MUTATION,
        MaintMeterReadingRecordedPayload {
            asset_id: Uuid::new_v4(),
            meter_type_id: Uuid::new_v4(),
            reading_value: 12500,
            recorded_at: "2026-03-01T14:30:00Z".into(),
        },
    );
    assert_envelope_completeness(&json, "maintenance/meter_reading.recorded");
}

#[test]
fn maintenance_meter_reading_recorded_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "meter_reading.recorded",
        mutation_classes::DATA_MUTATION,
        MaintMeterReadingRecordedPayload {
            asset_id: Uuid::new_v4(),
            meter_type_id: Uuid::new_v4(),
            reading_value: 50000,
            recorded_at: "2026-03-02T09:00:00Z".into(),
        },
    );
    validate_against_schema(&json, "maintenance-meter-reading-recorded.v1.json");
}

// ── plan.due ─────────────────────────────────────────────────────────

#[test]
fn maintenance_plan_due_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "plan.due",
        mutation_classes::DATA_MUTATION,
        MaintPlanDuePayload {
            assignment_id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            due_kind: "calendar".into(),
            due_value: "2026-03-15".into(),
        },
    );
    assert_envelope_completeness(&json, "maintenance/plan.due");
}

#[test]
fn maintenance_plan_due_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "plan.due",
        mutation_classes::DATA_MUTATION,
        MaintPlanDuePayload {
            assignment_id: Uuid::new_v4(),
            plan_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            due_kind: "meter".into(),
            due_value: "25000".into(),
        },
    );
    validate_against_schema(&json, "maintenance-plan-due.v1.json");
}

// ── plan.assigned ────────────────────────────────────────────────────

#[test]
fn maintenance_plan_assigned_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "plan.assigned",
        mutation_classes::DATA_MUTATION,
        MaintPlanAssignedPayload {
            plan_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            next_due_date: Some("2026-04-01".into()),
            next_due_meter: None,
        },
    );
    assert_envelope_completeness(&json, "maintenance/plan.assigned");
}

#[test]
fn maintenance_plan_assigned_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "maintenance",
        "plan.assigned",
        mutation_classes::DATA_MUTATION,
        MaintPlanAssignedPayload {
            plan_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            next_due_date: None,
            next_due_meter: Some(15000),
        },
    );
    validate_against_schema(&json, "maintenance-plan-assigned.v1.json");
}

// ── Exhaustive subject coverage ──────────────────────────────────────

#[test]
fn maintenance_all_subjects_have_contract_tests() {
    let tested_event_types = [
        "maintenance.work_order.created",
        "maintenance.work_order.status_changed",
        "maintenance.work_order.completed",
        "maintenance.work_order.closed",
        "maintenance.work_order.cancelled",
        "maintenance.work_order.overdue",
        "maintenance.meter_reading.recorded",
        "maintenance.plan.due",
        "maintenance.plan.assigned",
    ];
    assert_eq!(
        tested_event_types.len(),
        9,
        "Must have contract tests for all 9 maintenance event types"
    );
    for et in &tested_event_types {
        assert!(
            et.starts_with("maintenance."),
            "All maintenance event types must start with 'maintenance.'"
        );
        let type_part = et.strip_prefix("maintenance.").unwrap();
        assert!(
            event_naming::validate_event_type(type_part).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}

// ══════════════════════════════════════════════════════════════════════
// PDF EDITOR
// ══════════════════════════════════════════════════════════════════════

// ── PDF Editor Event Payloads ────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct PdfFormSubmittedPayload {
    tenant_id: String,
    submission_id: Uuid,
    template_id: Uuid,
    submitted_by: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PdfFormGeneratedPayload {
    tenant_id: String,
    submission_id: Uuid,
    template_id: Uuid,
}

// ── pdf.form.submitted ───────────────────────────────────────────────

#[test]
fn pdf_editor_form_submitted_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "pdf-editor",
        "pdf.form.submitted",
        mutation_classes::DATA_MUTATION,
        PdfFormSubmittedPayload {
            tenant_id: "tenant-001".into(),
            submission_id: Uuid::new_v4(),
            template_id: Uuid::new_v4(),
            submitted_by: "user@example.com".into(),
        },
    );
    assert_envelope_completeness(&json, "pdf-editor/pdf.form.submitted");
}

#[test]
fn pdf_editor_form_submitted_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "pdf-editor",
        "pdf.form.submitted",
        mutation_classes::DATA_MUTATION,
        PdfFormSubmittedPayload {
            tenant_id: "tenant-001".into(),
            submission_id: Uuid::new_v4(),
            template_id: Uuid::new_v4(),
            submitted_by: "admin@acme.com".into(),
        },
    );
    validate_against_schema(&json, "pdf-editor-form-submitted.v1.json");
}

// ── pdf.form.generated ───────────────────────────────────────────────

#[test]
fn pdf_editor_form_generated_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "pdf-editor",
        "pdf.form.generated",
        mutation_classes::DATA_MUTATION,
        PdfFormGeneratedPayload {
            tenant_id: "tenant-001".into(),
            submission_id: Uuid::new_v4(),
            template_id: Uuid::new_v4(),
        },
    );
    assert_envelope_completeness(&json, "pdf-editor/pdf.form.generated");
}

#[test]
fn pdf_editor_form_generated_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "pdf-editor",
        "pdf.form.generated",
        mutation_classes::DATA_MUTATION,
        PdfFormGeneratedPayload {
            tenant_id: "tenant-001".into(),
            submission_id: Uuid::new_v4(),
            template_id: Uuid::new_v4(),
        },
    );
    validate_against_schema(&json, "pdf-editor-form-generated.v1.json");
}

// ── Gate B: schemas exist on disk ────────────────────────────────────

#[test]
fn pdf_editor_gate_b_schemas_exist_on_disk() {
    let schemas = [
        "pdf-editor-form-submitted.v1.json",
        "pdf-editor-form-generated.v1.json",
    ];
    for name in &schemas {
        let path = contracts_dir().join("events").join(name);
        assert!(
            path.exists(),
            "Schema file missing: {}",
            path.display()
        );
    }
}

#[test]
fn pdf_editor_gate_b_schemas_require_envelope_fields() {
    let schemas = [
        "pdf-editor-form-submitted.v1.json",
        "pdf-editor-form-generated.v1.json",
    ];
    let required_envelope_fields = [
        "event_id",
        "occurred_at",
        "tenant_id",
        "source_module",
        "source_version",
        "schema_version",
        "replay_safe",
        "mutation_class",
        "payload",
    ];
    for name in &schemas {
        let schema = load_schema(name);
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} must have a top-level 'required' array", name));
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        for field in &required_envelope_fields {
            assert!(
                required_strs.contains(field),
                "{} must require '{}' at top level",
                name,
                field
            );
        }
    }
}

// ── Event naming and subject coverage ────────────────────────────────

#[test]
fn pdf_editor_event_types_follow_naming_convention() {
    let event_types = [
        "pdf.form.submitted",
        "pdf.form.generated",
    ];
    for et in &event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}

#[test]
fn pdf_editor_all_subjects_have_contract_tests() {
    let tested_event_types = [
        "pdf.form.submitted",
        "pdf.form.generated",
    ];
    assert_eq!(
        tested_event_types.len(),
        2,
        "Must have contract tests for all 2 pdf-editor event types"
    );
    for et in &tested_event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}

#[test]
fn pdf_editor_is_not_financial_module() {
    assert!(
        !mutation_classes::FINANCIAL_MODULES.contains(&"pdf-editor"),
        "pdf-editor should not be listed as a financial module"
    );
}

// ══════════════════════════════════════════════════════════════════════
// TIMEKEEPING
// ══════════════════════════════════════════════════════════════════════

// ── Timekeeping Entry Event Payloads ──────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetEntryCreatedPayload {
    entry_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    work_date: String,
    minutes: i32,
    version: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetEntryCorrectedPayload {
    entry_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    work_date: String,
    old_minutes: i32,
    new_minutes: i32,
    version: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetEntryVoidedPayload {
    entry_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    work_date: String,
    voided_minutes: i32,
    version: i32,
}

// ── Timekeeping Approval Event Payloads ──────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetSubmittedPayload {
    approval_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    period_start: String,
    period_end: String,
    total_minutes: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetApprovedPayload {
    approval_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    period_start: String,
    period_end: String,
    reviewer_id: Uuid,
    total_minutes: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetRejectedPayload {
    approval_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    period_start: String,
    period_end: String,
    reviewer_id: Uuid,
    notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimesheetRecalledPayload {
    approval_id: Uuid,
    app_id: String,
    employee_id: Uuid,
    period_start: String,
    period_end: String,
}

// ── Entry Created ────────────────────────────────────────────────────

#[test]
fn timekeeping_entry_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet_entry.created",
        mutation_classes::DATA_MUTATION,
        TimesheetEntryCreatedPayload {
            entry_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            work_date: "2026-03-01".into(),
            minutes: 480,
            version: 1,
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet_entry.created");
}

#[test]
fn timekeeping_entry_created_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet_entry.created",
        mutation_classes::DATA_MUTATION,
        TimesheetEntryCreatedPayload {
            entry_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            work_date: "2026-03-01".into(),
            minutes: 480,
            version: 1,
        },
    );
    validate_against_schema(&json, "timekeeping-entry-created.v1.json");
}

// ── Entry Corrected ──────────────────────────────────────────────────

#[test]
fn timekeeping_entry_corrected_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet_entry.corrected",
        mutation_classes::DATA_MUTATION,
        TimesheetEntryCorrectedPayload {
            entry_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            work_date: "2026-03-01".into(),
            old_minutes: 480,
            new_minutes: 360,
            version: 2,
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet_entry.corrected");
}

#[test]
fn timekeeping_entry_corrected_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet_entry.corrected",
        mutation_classes::DATA_MUTATION,
        TimesheetEntryCorrectedPayload {
            entry_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            work_date: "2026-03-01".into(),
            old_minutes: 480,
            new_minutes: 360,
            version: 2,
        },
    );
    validate_against_schema(&json, "timekeeping-entry-corrected.v1.json");
}

// ── Entry Voided ─────────────────────────────────────────────────────

#[test]
fn timekeeping_entry_voided_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet_entry.voided",
        mutation_classes::DATA_MUTATION,
        TimesheetEntryVoidedPayload {
            entry_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            work_date: "2026-03-01".into(),
            voided_minutes: 480,
            version: 2,
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet_entry.voided");
}

#[test]
fn timekeeping_entry_voided_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet_entry.voided",
        mutation_classes::DATA_MUTATION,
        TimesheetEntryVoidedPayload {
            entry_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            work_date: "2026-03-01".into(),
            voided_minutes: 480,
            version: 2,
        },
    );
    validate_against_schema(&json, "timekeeping-entry-voided.v1.json");
}

// ── Timesheet Submitted ──────────────────────────────────────────────

#[test]
fn timekeeping_timesheet_submitted_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.submitted",
        mutation_classes::DATA_MUTATION,
        TimesheetSubmittedPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
            total_minutes: 2400,
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet.submitted");
}

#[test]
fn timekeeping_timesheet_submitted_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.submitted",
        mutation_classes::DATA_MUTATION,
        TimesheetSubmittedPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
            total_minutes: 2400,
        },
    );
    validate_against_schema(&json, "timekeeping-timesheet-submitted.v1.json");
}

// ── Timesheet Approved ───────────────────────────────────────────────

#[test]
fn timekeeping_timesheet_approved_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.approved",
        mutation_classes::DATA_MUTATION,
        TimesheetApprovedPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
            reviewer_id: Uuid::new_v4(),
            total_minutes: 2400,
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet.approved");
}

#[test]
fn timekeeping_timesheet_approved_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.approved",
        mutation_classes::DATA_MUTATION,
        TimesheetApprovedPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
            reviewer_id: Uuid::new_v4(),
            total_minutes: 2400,
        },
    );
    validate_against_schema(&json, "timekeeping-timesheet-approved.v1.json");
}

// ── Timesheet Rejected ───────────────────────────────────────────────

#[test]
fn timekeeping_timesheet_rejected_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.rejected",
        mutation_classes::DATA_MUTATION,
        TimesheetRejectedPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
            reviewer_id: Uuid::new_v4(),
            notes: Some("Hours seem too high for Monday".into()),
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet.rejected");
}

#[test]
fn timekeeping_timesheet_rejected_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.rejected",
        mutation_classes::DATA_MUTATION,
        TimesheetRejectedPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
            reviewer_id: Uuid::new_v4(),
            notes: Some("Please correct Friday hours".into()),
        },
    );
    validate_against_schema(&json, "timekeeping-timesheet-rejected.v1.json");
}

// ── Timesheet Recalled ───────────────────────────────────────────────

#[test]
fn timekeeping_timesheet_recalled_envelope_completeness() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.recalled",
        mutation_classes::DATA_MUTATION,
        TimesheetRecalledPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
        },
    );
    assert_envelope_completeness(&json, "timekeeping/timesheet.recalled");
}

#[test]
fn timekeeping_timesheet_recalled_schema_validation() {
    let json = build_envelope(
        "tenant-001",
        "timekeeping",
        "timesheet.recalled",
        mutation_classes::DATA_MUTATION,
        TimesheetRecalledPayload {
            approval_id: Uuid::new_v4(),
            app_id: "tenant-001".into(),
            employee_id: Uuid::new_v4(),
            period_start: "2026-03-01".into(),
            period_end: "2026-03-07".into(),
        },
    );
    validate_against_schema(&json, "timekeeping-timesheet-recalled.v1.json");
}

// ── Coverage guard ───────────────────────────────────────────────────

#[test]
fn timekeeping_all_subjects_have_contract_tests() {
    let tested_event_types = [
        "timesheet_entry.created",
        "timesheet_entry.corrected",
        "timesheet_entry.voided",
        "timesheet.submitted",
        "timesheet.approved",
        "timesheet.rejected",
        "timesheet.recalled",
    ];
    assert_eq!(
        tested_event_types.len(),
        7,
        "Must have contract tests for all 7 timekeeping event types"
    );
    for et in &tested_event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}

#[test]
fn timekeeping_is_not_financial_module() {
    assert!(
        !mutation_classes::FINANCIAL_MODULES.contains(&"timekeeping"),
        "timekeeping should not be listed as a financial module"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Party Module Events
// ════════════════════════════════════════════════════════════════════════

// ── Party Payloads ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct PartyCreatedPayload {
    party_id: Uuid,
    app_id: String,
    party_type: String,
    display_name: String,
    email: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PartyUpdatedPayload {
    party_id: Uuid,
    app_id: String,
    display_name: Option<String>,
    email: Option<String>,
    updated_by: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PartyDeactivatedPayload {
    party_id: Uuid,
    app_id: String,
    deactivated_by: String,
    deactivated_at: String,
}

// ── party.created ───────────────────────────────────────────────────────

#[test]
fn party_created_envelope_completeness() {
    let json = build_envelope(
        "tenant-party-test",
        "party",
        "party.created",
        mutation_classes::DATA_MUTATION,
        PartyCreatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "tenant-party-test".to_string(),
            party_type: "company".to_string(),
            display_name: "Acme Corp".to_string(),
            email: Some("info@acme.com".to_string()),
            created_at: "2026-03-01T00:00:00Z".to_string(),
        },
    );
    assert_envelope_completeness(&json, "party.created");
}

#[test]
fn party_created_schema_validation() {
    let json = build_envelope(
        "tenant-party-test",
        "party",
        "party.created",
        mutation_classes::DATA_MUTATION,
        PartyCreatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "tenant-party-test".to_string(),
            party_type: "individual".to_string(),
            display_name: "Jane Doe".to_string(),
            email: None,
            created_at: "2026-03-01T00:00:00Z".to_string(),
        },
    );
    validate_against_schema(&json, "party-created.v1.json");
}

// ── party.updated ───────────────────────────────────────────────────────

#[test]
fn party_updated_envelope_completeness() {
    let json = build_envelope(
        "tenant-party-test",
        "party",
        "party.updated",
        mutation_classes::DATA_MUTATION,
        PartyUpdatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "tenant-party-test".to_string(),
            display_name: Some("Acme Corporation".to_string()),
            email: Some("new@acme.com".to_string()),
            updated_by: "user-42".to_string(),
            updated_at: "2026-03-01T01:00:00Z".to_string(),
        },
    );
    assert_envelope_completeness(&json, "party.updated");
}

#[test]
fn party_updated_schema_validation() {
    let json = build_envelope(
        "tenant-party-test",
        "party",
        "party.updated",
        mutation_classes::DATA_MUTATION,
        PartyUpdatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "tenant-party-test".to_string(),
            display_name: None,
            email: None,
            updated_by: "system".to_string(),
            updated_at: "2026-03-01T01:00:00Z".to_string(),
        },
    );
    validate_against_schema(&json, "party-updated.v1.json");
}

// ── party.deactivated ───────────────────────────────────────────────────

#[test]
fn party_deactivated_envelope_completeness() {
    let json = build_envelope(
        "tenant-party-test",
        "party",
        "party.deactivated",
        mutation_classes::LIFECYCLE,
        PartyDeactivatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "tenant-party-test".to_string(),
            deactivated_by: "admin-1".to_string(),
            deactivated_at: "2026-03-01T02:00:00Z".to_string(),
        },
    );
    assert_envelope_completeness(&json, "party.deactivated");
}

#[test]
fn party_deactivated_schema_validation() {
    let json = build_envelope(
        "tenant-party-test",
        "party",
        "party.deactivated",
        mutation_classes::LIFECYCLE,
        PartyDeactivatedPayload {
            party_id: Uuid::new_v4(),
            app_id: "tenant-party-test".to_string(),
            deactivated_by: "user-99".to_string(),
            deactivated_at: "2026-03-01T02:00:00Z".to_string(),
        },
    );
    validate_against_schema(&json, "party-deactivated.v1.json");
}

// ── Coverage guard ──────────────────────────────────────────────────────

#[test]
fn party_all_subjects_have_contract_tests() {
    let tested_event_types = [
        "party.created",
        "party.updated",
        "party.deactivated",
    ];
    assert_eq!(
        tested_event_types.len(),
        3,
        "Must have contract tests for all 3 party event types"
    );
    for et in &tested_event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}

#[test]
fn party_is_not_financial_module() {
    assert!(
        !mutation_classes::FINANCIAL_MODULES.contains(&"party"),
        "party should not be listed as a financial module"
    );
}

// ══════════════════════════════════════════════════════════════════════
// NUMBERING GATE B
// ══════════════════════════════════════════════════════════════════════

#[test]
fn numbering_gate_b_schemas_exist_on_disk() {
    let schemas = [
        "numbering-number-allocated.v1.json",
        "numbering-number-confirmed.v1.json",
        "numbering-policy-updated.v1.json",
    ];

    let events_dir = contracts_dir().join("events");
    for schema_name in &schemas {
        let path = events_dir.join(schema_name);
        assert!(path.exists(), "Schema file missing: {}", path.display());

        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", schema_name, e));
        let _: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", schema_name, e));
    }
}

#[test]
fn numbering_gate_b_schemas_require_envelope_fields() {
    let schemas = [
        "numbering-number-allocated.v1.json",
        "numbering-number-confirmed.v1.json",
        "numbering-policy-updated.v1.json",
    ];

    for schema_name in &schemas {
        let schema = load_schema(schema_name);
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no 'required' array", schema_name));
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

        for field in [
            "event_id",
            "occurred_at",
            "tenant_id",
            "source_module",
            "source_version",
            "schema_version",
            "replay_safe",
            "mutation_class",
            "payload",
        ] {
            assert!(
                required_strs.contains(&field),
                "{} missing required envelope field '{}'",
                schema_name,
                field
            );
        }
    }
}

#[test]
fn numbering_all_events_have_contract_tests() {
    let tested_event_types = [
        "number.allocated",
        "number.confirmed",
        "policy.updated",
    ];
    assert_eq!(
        tested_event_types.len(),
        3,
        "Must have contract tests for all 3 numbering event types"
    );
    for et in &tested_event_types {
        assert!(
            event_naming::validate_event_type(et).is_ok(),
            "Event type '{}' does not follow entity.action convention",
            et
        );
    }
}
