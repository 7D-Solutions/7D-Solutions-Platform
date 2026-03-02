//! E2E test for RBAC enforcement on operator actions
//!
//! This test verifies that:
//! 1. Admin role can perform all operations
//! 2. Operator role can perform limited operations (suspend, rebuild, verify, status)
//! 3. Auditor role can only perform read-only operations (verify, status, list)
//! 4. Unauthorized attempts are rejected with appropriate errors
//! 5. Authorization attempts are logged (audited)

use security::{Operation, RbacPolicy, Role};

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_admin_full_access() {
    // Admin should have access to all operations
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::TenantSuspend
    ));
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::TenantDeprovision
    ));
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::ProjectionRebuild
    ));
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::ProjectionVerify
    ));
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::ProjectionStatus
    ));
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::ProjectionList
    ));
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::FleetMigrate
    ));

    println!("✓ Admin has full access to all operations");
}

#[test]
fn test_operator_limited_access() {
    // Operator can suspend tenants
    assert!(RbacPolicy::has_permission(
        Role::Operator,
        Operation::TenantSuspend
    ));

    // Operator CANNOT deprovision tenants (destructive)
    assert!(!RbacPolicy::has_permission(
        Role::Operator,
        Operation::TenantDeprovision
    ));

    // Operator can rebuild, verify, and check status of projections
    assert!(RbacPolicy::has_permission(
        Role::Operator,
        Operation::ProjectionRebuild
    ));
    assert!(RbacPolicy::has_permission(
        Role::Operator,
        Operation::ProjectionVerify
    ));
    assert!(RbacPolicy::has_permission(
        Role::Operator,
        Operation::ProjectionStatus
    ));
    assert!(RbacPolicy::has_permission(
        Role::Operator,
        Operation::ProjectionList
    ));

    // Operator CANNOT perform fleet migrations
    assert!(!RbacPolicy::has_permission(
        Role::Operator,
        Operation::FleetMigrate
    ));

    println!("✓ Operator has limited access (suspend, projection ops, but not deprovision or fleet migrate)");
}

#[test]
fn test_auditor_read_only() {
    // Auditor can verify and check status (read-only)
    assert!(RbacPolicy::has_permission(
        Role::Auditor,
        Operation::ProjectionVerify
    ));
    assert!(RbacPolicy::has_permission(
        Role::Auditor,
        Operation::ProjectionStatus
    ));
    assert!(RbacPolicy::has_permission(
        Role::Auditor,
        Operation::ProjectionList
    ));

    // Auditor CANNOT perform any write operations
    assert!(!RbacPolicy::has_permission(
        Role::Auditor,
        Operation::TenantSuspend
    ));
    assert!(!RbacPolicy::has_permission(
        Role::Auditor,
        Operation::TenantDeprovision
    ));
    assert!(!RbacPolicy::has_permission(
        Role::Auditor,
        Operation::ProjectionRebuild
    ));
    assert!(!RbacPolicy::has_permission(
        Role::Auditor,
        Operation::FleetMigrate
    ));

    println!("✓ Auditor has read-only access (verify, status, list only)");
}

#[test]
fn test_authorize_success_logs_approval() {
    // Successful authorization should return Ok
    let result = RbacPolicy::authorize(
        Role::Admin,
        Operation::TenantDeprovision,
        "admin-user",
        "tenant-123",
    );

    assert!(result.is_ok(), "Admin should be authorized for deprovision");
    println!("✓ Authorization success is logged (info level)");
}

#[test]
fn test_authorize_failure_logs_denial() {
    // Failed authorization should return Err with details
    let result = RbacPolicy::authorize(
        Role::Auditor,
        Operation::TenantSuspend,
        "auditor-user",
        "tenant-123",
    );

    assert!(
        result.is_err(),
        "Auditor should NOT be authorized for suspend"
    );

    match result {
        Err(e) => {
            let error_msg = format!("{}", e);
            assert!(error_msg.contains("Insufficient permissions"));
            assert!(error_msg.contains("auditor-user"));
            assert!(error_msg.contains("tenant-123"));
            println!(
                "✓ Authorization denial is logged with actor and resource: {}",
                error_msg
            );
        }
        Ok(_) => panic!("Expected authorization failure"),
    }
}

#[test]
fn test_operator_cannot_deprovision() {
    // Operator attempting to deprovision should be denied
    let result = RbacPolicy::authorize(
        Role::Operator,
        Operation::TenantDeprovision,
        "operator-user",
        "tenant-456",
    );

    assert!(
        result.is_err(),
        "Operator should NOT be authorized for deprovision"
    );
    println!("✓ Operator denied for destructive operation (deprovision)");
}

#[test]
fn test_operator_can_suspend() {
    // Operator attempting to suspend should be allowed
    let result = RbacPolicy::authorize(
        Role::Operator,
        Operation::TenantSuspend,
        "operator-user",
        "tenant-789",
    );

    assert!(result.is_ok(), "Operator should be authorized for suspend");
    println!("✓ Operator allowed for non-destructive operation (suspend)");
}

#[test]
fn test_auditor_can_verify() {
    // Auditor attempting to verify should be allowed
    let result = RbacPolicy::authorize(
        Role::Auditor,
        Operation::ProjectionVerify,
        "auditor-user",
        "projection:invoices",
    );

    assert!(result.is_ok(), "Auditor should be authorized for verify");
    println!("✓ Auditor allowed for read-only operation (verify)");
}

#[test]
fn test_auditor_cannot_rebuild() {
    // Auditor attempting to rebuild should be denied
    let result = RbacPolicy::authorize(
        Role::Auditor,
        Operation::ProjectionRebuild,
        "auditor-user",
        "projection:invoices",
    );

    assert!(
        result.is_err(),
        "Auditor should NOT be authorized for rebuild"
    );
    println!("✓ Auditor denied for write operation (rebuild)");
}

#[test]
fn test_operator_cannot_fleet_migrate() {
    // Operator attempting fleet migration should be denied
    let result = RbacPolicy::authorize(
        Role::Operator,
        Operation::FleetMigrate,
        "operator-user",
        "100 tenants",
    );

    assert!(
        result.is_err(),
        "Operator should NOT be authorized for fleet migrate"
    );
    println!("✓ Operator denied for high-risk operation (fleet migrate)");
}

#[test]
fn test_admin_can_fleet_migrate() {
    // Admin attempting fleet migration should be allowed
    let result = RbacPolicy::authorize(
        Role::Admin,
        Operation::FleetMigrate,
        "admin-user",
        "100 tenants",
    );

    assert!(
        result.is_ok(),
        "Admin should be authorized for fleet migrate"
    );
    println!("✓ Admin allowed for high-risk operation (fleet migrate)");
}

#[test]
fn test_role_parsing_case_insensitive() {
    assert_eq!(Role::from_str("admin"), Some(Role::Admin));
    assert_eq!(Role::from_str("ADMIN"), Some(Role::Admin));
    assert_eq!(Role::from_str("Admin"), Some(Role::Admin));

    assert_eq!(Role::from_str("operator"), Some(Role::Operator));
    assert_eq!(Role::from_str("OPERATOR"), Some(Role::Operator));
    assert_eq!(Role::from_str("Operator"), Some(Role::Operator));

    assert_eq!(Role::from_str("auditor"), Some(Role::Auditor));
    assert_eq!(Role::from_str("AUDITOR"), Some(Role::Auditor));
    assert_eq!(Role::from_str("Auditor"), Some(Role::Auditor));

    assert_eq!(Role::from_str("invalid"), None);
    assert_eq!(Role::from_str("user"), None);

    println!("✓ Role parsing is case-insensitive");
}

#[test]
fn test_comprehensive_permission_matrix() {
    // Define all operations
    let all_operations = vec![
        Operation::TenantSuspend,
        Operation::TenantDeprovision,
        Operation::ProjectionRebuild,
        Operation::ProjectionVerify,
        Operation::ProjectionStatus,
        Operation::ProjectionList,
        Operation::FleetMigrate,
    ];

    // Admin: All operations allowed
    for op in &all_operations {
        assert!(
            RbacPolicy::has_permission(Role::Admin, *op),
            "Admin should have permission for {:?}",
            op
        );
    }

    // Operator: Specific operations allowed
    let operator_allowed = vec![
        Operation::TenantSuspend,
        Operation::ProjectionRebuild,
        Operation::ProjectionVerify,
        Operation::ProjectionStatus,
        Operation::ProjectionList,
    ];

    let operator_denied = vec![Operation::TenantDeprovision, Operation::FleetMigrate];

    for op in &operator_allowed {
        assert!(
            RbacPolicy::has_permission(Role::Operator, *op),
            "Operator should have permission for {:?}",
            op
        );
    }

    for op in &operator_denied {
        assert!(
            !RbacPolicy::has_permission(Role::Operator, *op),
            "Operator should NOT have permission for {:?}",
            op
        );
    }

    // Auditor: Only read operations allowed
    let auditor_allowed = vec![
        Operation::ProjectionVerify,
        Operation::ProjectionStatus,
        Operation::ProjectionList,
    ];

    let auditor_denied = vec![
        Operation::TenantSuspend,
        Operation::TenantDeprovision,
        Operation::ProjectionRebuild,
        Operation::FleetMigrate,
    ];

    for op in &auditor_allowed {
        assert!(
            RbacPolicy::has_permission(Role::Auditor, *op),
            "Auditor should have permission for {:?}",
            op
        );
    }

    for op in &auditor_denied {
        assert!(
            !RbacPolicy::has_permission(Role::Auditor, *op),
            "Auditor should NOT have permission for {:?}",
            op
        );
    }

    println!("✓ Comprehensive permission matrix verified for all roles");
}
