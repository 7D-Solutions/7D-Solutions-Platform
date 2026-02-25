//! Integration tests for audit policy enforcement.
//!
//! Verifies that validate_mutation correctly enforces StrictImmutable,
//! CompensatingRequired, MutableWithAudit, and MutableStandard policies.

use audit::policy::{
    validate_mutation, AuditPolicy, MutationOperation, MutationRequest, PolicyViolation,
};

fn make_request(
    policy: AuditPolicy,
    op: MutationOperation,
    has_linkage: bool,
    has_diff: bool,
) -> MutationRequest {
    MutationRequest {
        entity_type: "TestEntity".to_string(),
        entity_id: "te_1".to_string(),
        operation: op,
        policy,
        has_reversal_linkage: has_linkage,
        has_field_diff: has_diff,
    }
}

// ── Creates are always allowed ────────────────────────────────────────

#[test]
fn create_allowed_for_all_policies() {
    for policy in [
        AuditPolicy::StrictImmutable,
        AuditPolicy::CompensatingRequired,
        AuditPolicy::MutableWithAudit,
        AuditPolicy::MutableStandard,
    ] {
        let req = make_request(policy, MutationOperation::Create, false, false);
        assert!(
            validate_mutation(&req).is_ok(),
            "Create should be allowed for {policy:?}"
        );
    }
}

// ── StrictImmutable ───────────────────────────────────────────────────

#[test]
fn strict_immutable_blocks_update() {
    let req = make_request(
        AuditPolicy::StrictImmutable,
        MutationOperation::Update,
        false,
        false,
    );
    let err = validate_mutation(&req).unwrap_err();
    assert!(matches!(err, PolicyViolation::StrictImmutableViolation { .. }));
}

#[test]
fn strict_immutable_blocks_delete() {
    let req = make_request(
        AuditPolicy::StrictImmutable,
        MutationOperation::Delete,
        false,
        false,
    );
    let err = validate_mutation(&req).unwrap_err();
    assert!(matches!(err, PolicyViolation::DeleteNotAllowed { .. }));
}

// ── CompensatingRequired ──────────────────────────────────────────────

#[test]
fn compensating_rejects_update_without_linkage() {
    let req = make_request(
        AuditPolicy::CompensatingRequired,
        MutationOperation::Update,
        false,
        false,
    );
    let err = validate_mutation(&req).unwrap_err();
    assert!(matches!(
        err,
        PolicyViolation::MissingCompensationLinkage { .. }
    ));
}

#[test]
fn compensating_allows_update_with_linkage() {
    let req = make_request(
        AuditPolicy::CompensatingRequired,
        MutationOperation::Update,
        true,
        false,
    );
    assert!(validate_mutation(&req).is_ok());
}

#[test]
fn compensating_blocks_delete() {
    let req = make_request(
        AuditPolicy::CompensatingRequired,
        MutationOperation::Delete,
        false,
        false,
    );
    let err = validate_mutation(&req).unwrap_err();
    assert!(matches!(err, PolicyViolation::DeleteNotAllowed { .. }));
}

// ── MutableWithAudit ──────────────────────────────────────────────────

#[test]
fn mutable_with_audit_rejects_update_without_diff() {
    let req = make_request(
        AuditPolicy::MutableWithAudit,
        MutationOperation::Update,
        false,
        false,
    );
    let err = validate_mutation(&req).unwrap_err();
    assert!(matches!(err, PolicyViolation::MissingFieldDiff { .. }));
}

#[test]
fn mutable_with_audit_allows_update_with_diff() {
    let req = make_request(
        AuditPolicy::MutableWithAudit,
        MutationOperation::Update,
        false,
        true,
    );
    assert!(validate_mutation(&req).is_ok());
}

#[test]
fn mutable_with_audit_allows_delete() {
    let req = make_request(
        AuditPolicy::MutableWithAudit,
        MutationOperation::Delete,
        false,
        false,
    );
    assert!(validate_mutation(&req).is_ok());
}

// ── MutableStandard ───────────────────────────────────────────────────

#[test]
fn mutable_standard_allows_update_without_extras() {
    let req = make_request(
        AuditPolicy::MutableStandard,
        MutationOperation::Update,
        false,
        false,
    );
    assert!(validate_mutation(&req).is_ok());
}

#[test]
fn mutable_standard_allows_delete() {
    let req = make_request(
        AuditPolicy::MutableStandard,
        MutationOperation::Delete,
        false,
        false,
    );
    assert!(validate_mutation(&req).is_ok());
}

// ── Policy attribute checks ───────────────────────────────────────────

#[test]
fn policy_allows_direct_updates() {
    assert!(!AuditPolicy::StrictImmutable.allows_direct_updates());
    assert!(!AuditPolicy::CompensatingRequired.allows_direct_updates());
    assert!(AuditPolicy::MutableWithAudit.allows_direct_updates());
    assert!(AuditPolicy::MutableStandard.allows_direct_updates());
}

#[test]
fn policy_requires_compensation() {
    assert!(!AuditPolicy::StrictImmutable.requires_compensation());
    assert!(AuditPolicy::CompensatingRequired.requires_compensation());
    assert!(!AuditPolicy::MutableWithAudit.requires_compensation());
    assert!(!AuditPolicy::MutableStandard.requires_compensation());
}

#[test]
fn policy_requires_field_diff() {
    assert!(!AuditPolicy::StrictImmutable.requires_field_diff());
    assert!(!AuditPolicy::CompensatingRequired.requires_field_diff());
    assert!(AuditPolicy::MutableWithAudit.requires_field_diff());
    assert!(!AuditPolicy::MutableStandard.requires_field_diff());
}

// ── Error message formatting ──────────────────────────────────────────

#[test]
fn violation_errors_include_entity_info() {
    let req = MutationRequest {
        entity_type: "Invoice".to_string(),
        entity_id: "inv_999".to_string(),
        operation: MutationOperation::Update,
        policy: AuditPolicy::StrictImmutable,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let err = validate_mutation(&req).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invoice"), "error should contain entity type");
    assert!(msg.contains("inv_999"), "error should contain entity id");
}
