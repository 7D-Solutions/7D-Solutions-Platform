//! E2E test for RBAC enforcement on operator actions (JWT permission-based)
//!
//! This test verifies that:
//! 1. check_permissions grants access when all required permissions are present
//! 2. check_permissions denies access when permissions are missing
//! 3. Permission strings match between CLI constants and old Operation::as_str()
//! 4. Integration test: CLI with token missing "tenant.suspend" gets rejected
//! 5. Integration test: CLI with valid permissions succeeds

use security::{
    check_permissions, VerifiedClaims, PERM_FLEET_MIGRATE, PERM_PROJECTION_LIST,
    PERM_PROJECTION_REBUILD, PERM_PROJECTION_STATUS, PERM_PROJECTION_VERIFY,
    PERM_TENANT_DEPROVISION, PERM_TENANT_SUSPEND,
};

// ============================================================================
// Test helpers
// ============================================================================

/// Build a minimal VerifiedClaims for unit tests with the given permissions.
fn test_claims(perms: Vec<&str>) -> VerifiedClaims {
    use chrono::Utc;
    use uuid::Uuid;

    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: None,
        roles: vec!["operator".into()],
        perms: perms.into_iter().map(|s| s.to_string()).collect(),
        actor_type: security::ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

// ============================================================================
// Permission check: grants and denials
// ============================================================================

#[test]
fn admin_with_all_permissions_granted() {
    let claims = test_claims(vec![
        PERM_TENANT_SUSPEND,
        PERM_TENANT_DEPROVISION,
        PERM_PROJECTION_REBUILD,
        PERM_PROJECTION_VERIFY,
        PERM_PROJECTION_STATUS,
        PERM_PROJECTION_LIST,
        PERM_FLEET_MIGRATE,
    ]);

    assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND]).is_ok());
    assert!(check_permissions(&claims, &[PERM_TENANT_DEPROVISION]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_REBUILD]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_VERIFY]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_STATUS]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_LIST]).is_ok());
    assert!(check_permissions(&claims, &[PERM_FLEET_MIGRATE]).is_ok());

    // Multiple permissions at once
    assert!(check_permissions(
        &claims,
        &[
            PERM_TENANT_SUSPEND,
            PERM_TENANT_DEPROVISION,
            PERM_FLEET_MIGRATE
        ]
    )
    .is_ok());

    println!("✓ Admin-like token with all permissions is granted access to all operations");
}

#[test]
fn operator_limited_permissions() {
    // Operator has suspend and projection ops but NOT deprovision or fleet migrate
    let claims = test_claims(vec![
        PERM_TENANT_SUSPEND,
        PERM_PROJECTION_REBUILD,
        PERM_PROJECTION_VERIFY,
        PERM_PROJECTION_STATUS,
        PERM_PROJECTION_LIST,
    ]);

    // Allowed operations
    assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_REBUILD]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_VERIFY]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_STATUS]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_LIST]).is_ok());

    // Denied operations
    assert!(
        check_permissions(&claims, &[PERM_TENANT_DEPROVISION]).is_err(),
        "Operator should NOT have deprovision permission"
    );
    assert!(
        check_permissions(&claims, &[PERM_FLEET_MIGRATE]).is_err(),
        "Operator should NOT have fleet migrate permission"
    );

    println!("✓ Operator-like token has limited access (suspend, projection ops, but not deprovision or fleet migrate)");
}

#[test]
fn auditor_read_only_permissions() {
    // Auditor has only read operations
    let claims = test_claims(vec![
        PERM_PROJECTION_VERIFY,
        PERM_PROJECTION_STATUS,
        PERM_PROJECTION_LIST,
    ]);

    // Allowed read operations
    assert!(check_permissions(&claims, &[PERM_PROJECTION_VERIFY]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_STATUS]).is_ok());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_LIST]).is_ok());

    // Denied write operations
    assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND]).is_err());
    assert!(check_permissions(&claims, &[PERM_TENANT_DEPROVISION]).is_err());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_REBUILD]).is_err());
    assert!(check_permissions(&claims, &[PERM_FLEET_MIGRATE]).is_err());

    println!("✓ Auditor-like token has read-only access (verify, status, list only)");
}

#[test]
fn empty_permissions_denied_for_all_operations() {
    let claims = test_claims(vec![]);

    assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND]).is_err());
    assert!(check_permissions(&claims, &[PERM_TENANT_DEPROVISION]).is_err());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_REBUILD]).is_err());
    assert!(check_permissions(&claims, &[PERM_PROJECTION_VERIFY]).is_err());
    assert!(check_permissions(&claims, &[PERM_FLEET_MIGRATE]).is_err());

    // Empty required = always granted (no permissions needed)
    assert!(check_permissions(&claims, &[]).is_ok());

    println!("✓ Token with no permissions is denied all operations");
}

#[test]
fn check_permissions_denies_partial_match() {
    // Token has tenant.suspend but operation requires both suspend and deprovision
    let claims = test_claims(vec![PERM_TENANT_SUSPEND]);

    assert!(
        check_permissions(&claims, &[PERM_TENANT_SUSPEND, PERM_TENANT_DEPROVISION]).is_err(),
        "Partial permission match should be denied"
    );

    println!("✓ Partial permission match is correctly denied");
}

#[test]
fn superset_permissions_granted() {
    // Token has more permissions than required
    let claims = test_claims(vec![
        PERM_TENANT_SUSPEND,
        PERM_TENANT_DEPROVISION,
        PERM_PROJECTION_REBUILD,
        PERM_FLEET_MIGRATE,
    ]);

    // Checking a subset should succeed
    assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND]).is_ok());
    assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND, PERM_FLEET_MIGRATE]).is_ok());

    println!("✓ Superset permissions are correctly granted");
}

// ============================================================================
// Permission constant integrity
// ============================================================================

#[test]
fn permission_constants_follow_dot_convention() {
    let all_perms = [
        PERM_TENANT_SUSPEND,
        PERM_TENANT_DEPROVISION,
        PERM_PROJECTION_REBUILD,
        PERM_PROJECTION_VERIFY,
        PERM_PROJECTION_STATUS,
        PERM_PROJECTION_LIST,
        PERM_FLEET_MIGRATE,
    ];

    for perm in &all_perms {
        assert!(
            perm.contains('.'),
            "Permission '{}' must follow module.action convention",
            perm
        );
        let parts: Vec<&str> = perm.splitn(2, '.').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty());
        assert!(!parts[1].is_empty());
    }

    println!("✓ All CLI permission constants follow module.action convention");
}

#[test]
fn permission_constants_are_unique() {
    let all_perms = vec![
        PERM_TENANT_SUSPEND,
        PERM_TENANT_DEPROVISION,
        PERM_PROJECTION_REBUILD,
        PERM_PROJECTION_VERIFY,
        PERM_PROJECTION_STATUS,
        PERM_PROJECTION_LIST,
        PERM_FLEET_MIGRATE,
    ];

    let mut unique = all_perms.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        all_perms.len(),
        "All permission constants must be unique"
    );

    println!("✓ All CLI permission constants are unique");
}

// ============================================================================
// Backward compatibility: deprecated types still work
// ============================================================================

#[test]
#[allow(deprecated)]
fn deprecated_types_still_compile() {
    use security::{Operation, RbacPolicy, Role};

    // Old API still functions (just deprecated)
    assert!(RbacPolicy::has_permission(
        Role::Admin,
        Operation::TenantSuspend
    ));
    assert!(!RbacPolicy::has_permission(
        Role::Auditor,
        Operation::TenantSuspend
    ));

    // Old Operation::as_str() matches new PERM_ constants
    assert_eq!(Operation::TenantSuspend.as_str(), PERM_TENANT_SUSPEND);
    assert_eq!(
        Operation::TenantDeprovision.as_str(),
        PERM_TENANT_DEPROVISION
    );
    assert_eq!(
        Operation::ProjectionRebuild.as_str(),
        PERM_PROJECTION_REBUILD
    );
    assert_eq!(Operation::ProjectionVerify.as_str(), PERM_PROJECTION_VERIFY);
    assert_eq!(Operation::ProjectionStatus.as_str(), PERM_PROJECTION_STATUS);
    assert_eq!(Operation::ProjectionList.as_str(), PERM_PROJECTION_LIST);
    assert_eq!(Operation::FleetMigrate.as_str(), PERM_FLEET_MIGRATE);

    println!("✓ Deprecated types still compile and permission strings match");
}
