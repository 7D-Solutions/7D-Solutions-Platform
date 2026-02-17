//! E2E Test: Audit Policy Enforcement
//!
//! Validates that audit policies (strict_immutable, compensating_required, mutable_with_audit)
//! are enforced at mutation boundaries across modules.
//!
//! **Test Coverage:**
//! 1. StrictImmutable: Verify that updates to immutable entities are rejected
//! 2. CompensatingRequired: Verify that updates without reversal linkage are rejected
//! 3. MutableWithAudit: Verify that updates without field diff are rejected

use audit::policy::{
    validate_mutation, AuditPolicy, MutationOperation, MutationRequest, PolicyViolation,
};
use uuid::Uuid;

#[test]
fn test_strict_immutable_policy_enforcement() {
    // Test: StrictImmutable policy blocks updates
    let request = MutationRequest {
        entity_type: "Invoice".to_string(),
        entity_id: "inv_12345".to_string(),
        operation: MutationOperation::Update,
        policy: AuditPolicy::StrictImmutable,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(result.is_err(), "StrictImmutable should block updates");

    match result.unwrap_err() {
        PolicyViolation::StrictImmutableViolation {
            entity_type,
            entity_id,
        } => {
            assert_eq!(entity_type, "Invoice");
            assert_eq!(entity_id, "inv_12345");
        }
        other => panic!("Expected StrictImmutableViolation, got {:?}", other),
    }
}

#[test]
fn test_strict_immutable_allows_creates() {
    // Test: StrictImmutable policy allows creates
    let request = MutationRequest {
        entity_type: "Invoice".to_string(),
        entity_id: "inv_12345".to_string(),
        operation: MutationOperation::Create,
        policy: AuditPolicy::StrictImmutable,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(result.is_ok(), "StrictImmutable should allow creates");
}

#[test]
fn test_strict_immutable_blocks_deletes() {
    // Test: StrictImmutable policy blocks deletes
    let request = MutationRequest {
        entity_type: "Invoice".to_string(),
        entity_id: "inv_12345".to_string(),
        operation: MutationOperation::Delete,
        policy: AuditPolicy::StrictImmutable,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(result.is_err(), "StrictImmutable should block deletes");

    match result.unwrap_err() {
        PolicyViolation::DeleteNotAllowed { policy } => {
            assert_eq!(policy, AuditPolicy::StrictImmutable);
        }
        other => panic!("Expected DeleteNotAllowed, got {:?}", other),
    }
}

#[test]
fn test_compensating_required_without_linkage() {
    // Test: CompensatingRequired policy blocks updates without reversal linkage
    let request = MutationRequest {
        entity_type: "Payment".to_string(),
        entity_id: "pay_67890".to_string(),
        operation: MutationOperation::Update,
        policy: AuditPolicy::CompensatingRequired,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(
        result.is_err(),
        "CompensatingRequired should block updates without linkage"
    );

    match result.unwrap_err() {
        PolicyViolation::MissingCompensationLinkage {
            entity_type,
            entity_id,
        } => {
            assert_eq!(entity_type, "Payment");
            assert_eq!(entity_id, "pay_67890");
        }
        other => panic!("Expected MissingCompensationLinkage, got {:?}", other),
    }
}

#[test]
fn test_compensating_required_with_linkage() {
    // Test: CompensatingRequired policy allows updates with reversal linkage
    let request = MutationRequest {
        entity_type: "Payment".to_string(),
        entity_id: "pay_67890".to_string(),
        operation: MutationOperation::Update,
        policy: AuditPolicy::CompensatingRequired,
        has_reversal_linkage: true,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(
        result.is_ok(),
        "CompensatingRequired should allow updates with linkage"
    );
}

#[test]
fn test_compensating_required_blocks_deletes() {
    // Test: CompensatingRequired policy blocks deletes
    let request = MutationRequest {
        entity_type: "Payment".to_string(),
        entity_id: "pay_67890".to_string(),
        operation: MutationOperation::Delete,
        policy: AuditPolicy::CompensatingRequired,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(result.is_err(), "CompensatingRequired should block deletes");

    match result.unwrap_err() {
        PolicyViolation::DeleteNotAllowed { policy } => {
            assert_eq!(policy, AuditPolicy::CompensatingRequired);
        }
        other => panic!("Expected DeleteNotAllowed, got {:?}", other),
    }
}

#[test]
fn test_mutable_with_audit_without_diff() {
    // Test: MutableWithAudit policy blocks updates without field diff
    let request = MutationRequest {
        entity_type: "Customer".to_string(),
        entity_id: "cust_11111".to_string(),
        operation: MutationOperation::Update,
        policy: AuditPolicy::MutableWithAudit,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(
        result.is_err(),
        "MutableWithAudit should block updates without field diff"
    );

    match result.unwrap_err() {
        PolicyViolation::MissingFieldDiff {
            entity_type,
            entity_id,
        } => {
            assert_eq!(entity_type, "Customer");
            assert_eq!(entity_id, "cust_11111");
        }
        other => panic!("Expected MissingFieldDiff, got {:?}", other),
    }
}

#[test]
fn test_mutable_with_audit_with_diff() {
    // Test: MutableWithAudit policy allows updates with field diff
    let request = MutationRequest {
        entity_type: "Customer".to_string(),
        entity_id: "cust_11111".to_string(),
        operation: MutationOperation::Update,
        policy: AuditPolicy::MutableWithAudit,
        has_reversal_linkage: false,
        has_field_diff: true,
    };

    let result = validate_mutation(&request);
    assert!(
        result.is_ok(),
        "MutableWithAudit should allow updates with field diff"
    );
}

#[test]
fn test_mutable_with_audit_allows_deletes() {
    // Test: MutableWithAudit policy allows deletes
    let request = MutationRequest {
        entity_type: "Customer".to_string(),
        entity_id: "cust_11111".to_string(),
        operation: MutationOperation::Delete,
        policy: AuditPolicy::MutableWithAudit,
        has_reversal_linkage: false,
        has_field_diff: false,
    };

    let result = validate_mutation(&request);
    assert!(result.is_ok(), "MutableWithAudit should allow deletes");
}

#[test]
fn test_mutable_standard_allows_all_operations() {
    // Test: MutableStandard policy allows all operations without restrictions
    for operation in [
        MutationOperation::Create,
        MutationOperation::Update,
        MutationOperation::Delete,
    ] {
        let request = MutationRequest {
            entity_type: "TempData".to_string(),
            entity_id: "tmp_00000".to_string(),
            operation,
            policy: AuditPolicy::MutableStandard,
            has_reversal_linkage: false,
            has_field_diff: false,
        };

        let result = validate_mutation(&request);
        assert!(
            result.is_ok(),
            "MutableStandard should allow {:?} without restrictions",
            operation
        );
    }
}

#[test]
fn test_policy_enforcement_matrix() {
    // Test: Comprehensive policy enforcement matrix
    struct TestCase {
        policy: AuditPolicy,
        operation: MutationOperation,
        has_reversal_linkage: bool,
        has_field_diff: bool,
        should_succeed: bool,
        description: &'static str,
    }

    let test_cases = vec![
        // StrictImmutable
        TestCase {
            policy: AuditPolicy::StrictImmutable,
            operation: MutationOperation::Create,
            has_reversal_linkage: false,
            has_field_diff: false,
            should_succeed: true,
            description: "StrictImmutable allows creates",
        },
        TestCase {
            policy: AuditPolicy::StrictImmutable,
            operation: MutationOperation::Update,
            has_reversal_linkage: false,
            has_field_diff: false,
            should_succeed: false,
            description: "StrictImmutable blocks updates",
        },
        TestCase {
            policy: AuditPolicy::StrictImmutable,
            operation: MutationOperation::Delete,
            has_reversal_linkage: false,
            has_field_diff: false,
            should_succeed: false,
            description: "StrictImmutable blocks deletes",
        },
        // CompensatingRequired
        TestCase {
            policy: AuditPolicy::CompensatingRequired,
            operation: MutationOperation::Update,
            has_reversal_linkage: false,
            has_field_diff: false,
            should_succeed: false,
            description: "CompensatingRequired blocks updates without linkage",
        },
        TestCase {
            policy: AuditPolicy::CompensatingRequired,
            operation: MutationOperation::Update,
            has_reversal_linkage: true,
            has_field_diff: false,
            should_succeed: true,
            description: "CompensatingRequired allows updates with linkage",
        },
        // MutableWithAudit
        TestCase {
            policy: AuditPolicy::MutableWithAudit,
            operation: MutationOperation::Update,
            has_reversal_linkage: false,
            has_field_diff: false,
            should_succeed: false,
            description: "MutableWithAudit blocks updates without diff",
        },
        TestCase {
            policy: AuditPolicy::MutableWithAudit,
            operation: MutationOperation::Update,
            has_reversal_linkage: false,
            has_field_diff: true,
            should_succeed: true,
            description: "MutableWithAudit allows updates with diff",
        },
        // MutableStandard
        TestCase {
            policy: AuditPolicy::MutableStandard,
            operation: MutationOperation::Update,
            has_reversal_linkage: false,
            has_field_diff: false,
            should_succeed: true,
            description: "MutableStandard allows updates unconditionally",
        },
    ];

    for (i, test_case) in test_cases.iter().enumerate() {
        let request = MutationRequest {
            entity_type: "TestEntity".to_string(),
            entity_id: format!("test_{}", i),
            operation: test_case.operation,
            policy: test_case.policy,
            has_reversal_linkage: test_case.has_reversal_linkage,
            has_field_diff: test_case.has_field_diff,
        };

        let result = validate_mutation(&request);
        assert_eq!(
            result.is_ok(),
            test_case.should_succeed,
            "Test case {} failed: {}",
            i,
            test_case.description
        );
    }
}
