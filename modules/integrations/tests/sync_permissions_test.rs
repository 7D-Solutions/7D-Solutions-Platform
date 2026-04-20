//! Sync permission constants and router structure tests (bd-n68o6)
//!
//! Proves:
//! 1. All four sync permission constants have the expected string values and are distinct
//! 2. Sync permissions are not interchangeable with the coarser integrations.mutate/read
//! 3. Router builds with the new sync route groups (compile-time check)

// ============================================================================
// 1. Sync permission constants — values and distinctness
// ============================================================================

#[test]
fn sync_permission_constants_have_correct_values() {
    assert_eq!(
        security::permissions::INTEGRATIONS_SYNC_AUTHORITY_FLIP,
        "integrations.sync.authority.flip"
    );
    assert_eq!(
        security::permissions::INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
        "integrations.sync.conflict.resolve"
    );
    assert_eq!(
        security::permissions::INTEGRATIONS_SYNC_PUSH,
        "integrations.sync.push"
    );
    assert_eq!(
        security::permissions::INTEGRATIONS_SYNC_READ,
        "integrations.sync.read"
    );
}

#[test]
fn sync_permission_constants_are_non_empty() {
    assert!(!security::permissions::INTEGRATIONS_SYNC_AUTHORITY_FLIP.is_empty());
    assert!(!security::permissions::INTEGRATIONS_SYNC_CONFLICT_RESOLVE.is_empty());
    assert!(!security::permissions::INTEGRATIONS_SYNC_PUSH.is_empty());
    assert!(!security::permissions::INTEGRATIONS_SYNC_READ.is_empty());
}

#[test]
fn sync_permission_constants_are_distinct() {
    let perms = [
        security::permissions::INTEGRATIONS_SYNC_AUTHORITY_FLIP,
        security::permissions::INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
        security::permissions::INTEGRATIONS_SYNC_PUSH,
        security::permissions::INTEGRATIONS_SYNC_READ,
    ];
    for (i, a) in perms.iter().enumerate() {
        for (j, b) in perms.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "sync permissions[{}] and [{}] must differ", i, j);
            }
        }
    }
}

// ============================================================================
// 2. Sync permissions do not overlap with coarser integrations permissions
// ============================================================================

#[test]
fn sync_permissions_not_interchangeable_with_coarse_permissions() {
    assert_ne!(
        security::permissions::INTEGRATIONS_SYNC_AUTHORITY_FLIP,
        security::permissions::INTEGRATIONS_MUTATE,
        "authority.flip must not equal integrations.mutate"
    );
    assert_ne!(
        security::permissions::INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
        security::permissions::INTEGRATIONS_MUTATE,
        "conflict.resolve must not equal integrations.mutate"
    );
    assert_ne!(
        security::permissions::INTEGRATIONS_SYNC_PUSH,
        security::permissions::INTEGRATIONS_MUTATE,
        "sync.push must not equal integrations.mutate"
    );
    assert_ne!(
        security::permissions::INTEGRATIONS_SYNC_READ,
        security::permissions::INTEGRATIONS_READ,
        "sync.read must not equal integrations.read"
    );
}

#[test]
fn sync_permissions_follow_dot_convention() {
    let perms = [
        security::permissions::INTEGRATIONS_SYNC_AUTHORITY_FLIP,
        security::permissions::INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
        security::permissions::INTEGRATIONS_SYNC_PUSH,
        security::permissions::INTEGRATIONS_SYNC_READ,
    ];
    for perm in &perms {
        assert!(
            perm.starts_with("integrations.sync."),
            "Sync permission '{}' must be under integrations.sync namespace",
            perm
        );
    }
}

// ============================================================================
// 3. Router builds with sync route groups (compile-time check)
// ============================================================================

#[test]
fn router_builds_with_sync_routes() {
    let _: fn(std::sync::Arc<integrations_rs::AppState>) -> axum::Router =
        integrations_rs::http::router;
}
