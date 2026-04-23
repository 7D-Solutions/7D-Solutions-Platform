//! Integration tests for the feature flag framework.
//!
//! These tests run against a real PostgreSQL database.  Set
//! `TENANT_REGISTRY_DATABASE_URL` to point at the tenant-registry database
//! (which hosts the `feature_flags` table after the migration is applied).
//!
//! Run with:
//!   cargo test -p feature-flags --test integration -- --nocapture

use feature_flags::{delete_flag, is_enabled, list_flags_for_tenant, set_flag};
use sqlx::PgPool;
use uuid::Uuid;

async fn test_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    PgPool::connect(&url)
        .await
        .expect("connect to tenant-registry database for feature-flags tests")
}

/// Absent flag defaults to false.
#[tokio::test]
async fn feature_flag_absent_is_disabled() {
    let pool = test_pool().await;
    let flag = format!("test_absent_{}", Uuid::new_v4().simple());
    let tenant = Uuid::new_v4();

    let global = is_enabled(&pool, &flag, None).await.unwrap();
    let scoped = is_enabled(&pool, &flag, Some(tenant)).await.unwrap();

    assert!(!global, "absent global flag must be disabled");
    assert!(!scoped, "absent per-tenant flag must be disabled");
}

/// Global flag enables for all tenants when no per-tenant override exists.
#[tokio::test]
async fn feature_flag_global_enables_for_all_tenants() {
    let pool = test_pool().await;
    let flag = format!("test_global_{}", Uuid::new_v4().simple());
    let tenant = Uuid::new_v4();

    set_flag(&pool, &flag, None, true).await.unwrap();

    let global = is_enabled(&pool, &flag, None).await.unwrap();
    let scoped = is_enabled(&pool, &flag, Some(tenant)).await.unwrap();

    assert!(
        global,
        "enabled global flag must be enabled when queried globally"
    );
    assert!(
        scoped,
        "enabled global flag must be enabled for a tenant with no override"
    );

    // Cleanup.
    delete_flag(&pool, &flag, None).await.unwrap();
}

/// Per-tenant override disables a globally-enabled flag for that tenant.
#[tokio::test]
async fn feature_flag_per_tenant_disables_global() {
    let pool = test_pool().await;
    let flag = format!("test_override_{}", Uuid::new_v4().simple());
    let tenant = Uuid::new_v4();

    // Enable globally.
    set_flag(&pool, &flag, None, true).await.unwrap();

    // Disable for the specific tenant.
    set_flag(&pool, &flag, Some(tenant), false).await.unwrap();

    let global = is_enabled(&pool, &flag, None).await.unwrap();
    let scoped = is_enabled(&pool, &flag, Some(tenant)).await.unwrap();

    assert!(global, "global flag must remain enabled");
    assert!(
        !scoped,
        "per-tenant override must disable the flag for that tenant"
    );

    // Cleanup.
    delete_flag(&pool, &flag, None).await.unwrap();
    delete_flag(&pool, &flag, Some(tenant)).await.unwrap();
}

/// Per-tenant override enables a globally-disabled flag for that tenant.
#[tokio::test]
async fn feature_flag_per_tenant_enables_global_disabled() {
    let pool = test_pool().await;
    let flag = format!("test_opt_in_{}", Uuid::new_v4().simple());
    let tenant = Uuid::new_v4();

    // Disable globally (explicit).
    set_flag(&pool, &flag, None, false).await.unwrap();

    // Enable for the specific tenant (beta opt-in).
    set_flag(&pool, &flag, Some(tenant), true).await.unwrap();

    let global = is_enabled(&pool, &flag, None).await.unwrap();
    let scoped = is_enabled(&pool, &flag, Some(tenant)).await.unwrap();

    assert!(!global, "global flag must remain disabled");
    assert!(
        scoped,
        "per-tenant opt-in must enable the flag for that tenant"
    );

    // Cleanup.
    delete_flag(&pool, &flag, None).await.unwrap();
    delete_flag(&pool, &flag, Some(tenant)).await.unwrap();
}

/// Updating a flag via set_flag is idempotent (upsert).
#[tokio::test]
async fn feature_flag_upsert_is_idempotent() {
    let pool = test_pool().await;
    let flag = format!("test_upsert_{}", Uuid::new_v4().simple());

    set_flag(&pool, &flag, None, true).await.unwrap();
    set_flag(&pool, &flag, None, true).await.unwrap(); // second call must not error
    set_flag(&pool, &flag, None, false).await.unwrap();

    let enabled = is_enabled(&pool, &flag, None).await.unwrap();
    assert!(!enabled, "flag must reflect the last set_flag call");

    delete_flag(&pool, &flag, None).await.unwrap();
}

/// list_flags_for_tenant returns per-tenant row when it exists, global row otherwise.
#[tokio::test]
async fn list_flags_for_tenant_returns_per_tenant_and_global_flags() {
    let pool = test_pool().await;
    let tenant = Uuid::new_v4();
    let global_flag = format!("test_list_global_{}", Uuid::new_v4().simple());
    let tenant_flag = format!("test_list_tenant_{}", Uuid::new_v4().simple());

    // Set a global flag and a per-tenant override
    set_flag(&pool, &global_flag, None, true).await.unwrap();
    set_flag(&pool, &tenant_flag, Some(tenant), false)
        .await
        .unwrap();

    let flags = list_flags_for_tenant(&pool, tenant).await.unwrap();

    assert_eq!(
        flags.get(&global_flag),
        Some(&true),
        "global flag must appear"
    );
    assert_eq!(
        flags.get(&tenant_flag),
        Some(&false),
        "per-tenant flag must appear"
    );

    // Cleanup
    delete_flag(&pool, &global_flag, None).await.unwrap();
    delete_flag(&pool, &tenant_flag, Some(tenant))
        .await
        .unwrap();
}

/// list_flags_for_tenant: per-tenant row overrides global when both exist.
#[tokio::test]
async fn list_flags_for_tenant_per_tenant_overrides_global() {
    let pool = test_pool().await;
    let tenant = Uuid::new_v4();
    let flag = format!("test_list_override_{}", Uuid::new_v4().simple());

    // Global says true, per-tenant says false
    set_flag(&pool, &flag, None, true).await.unwrap();
    set_flag(&pool, &flag, Some(tenant), false).await.unwrap();

    let flags = list_flags_for_tenant(&pool, tenant).await.unwrap();
    assert_eq!(
        flags.get(&flag),
        Some(&false),
        "per-tenant override must win"
    );

    // Cleanup
    delete_flag(&pool, &flag, None).await.unwrap();
    delete_flag(&pool, &flag, Some(tenant)).await.unwrap();
}
