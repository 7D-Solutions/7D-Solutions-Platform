//! E2E tests for full tenant provisioning lifecycle: seed + health + activation
//!
//! These tests verify:
//! - Seed data is correctly inserted into each module's database
//! - Tenant activation is atomic: status and outbox event in one transaction
//! - Health checks gate activation (modules not ready → tenant stays provisioning)
//! - tenant.provisioned outbox event is written on successful activation

mod common;

use common::{
    get_ar_pool, get_auth_pool, get_gl_pool, get_subscriptions_pool, get_tenant_registry_pool,
};
use sqlx::PgPool;
use tenant_registry::{
    activate_tenant_atomic, check_all_modules_ready, seed_ar_module, seed_gl_module,
    seed_identity_module, seed_subscriptions_module, ModuleUrl,
};
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

/// Insert a test tenant in 'provisioning' state
async fn insert_provisioning_tenant(registry_pool: &PgPool, tenant_id: Uuid) {
    sqlx::query(
        r#"
        INSERT INTO tenants (tenant_id, status, environment, module_schema_versions)
        VALUES ($1, 'provisioning', 'development', '{}'::jsonb)
        ON CONFLICT (tenant_id) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(registry_pool)
    .await
    .expect("Failed to insert test tenant");
}

async fn get_tenant_status(registry_pool: &PgPool, tenant_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(registry_pool)
        .await
        .expect("Failed to fetch tenant status")
}

async fn get_outbox_event_count(registry_pool: &PgPool, tenant_id: Uuid, event_type: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM provisioning_outbox
        WHERE tenant_id = $1
          AND event_type = $2
        "#,
    )
    .bind(tenant_id)
    .bind(event_type)
    .fetch_one(registry_pool)
    .await
    .expect("Failed to count outbox events")
}

async fn cleanup_tenant(registry_pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(registry_pool)
        .await
        .ok();
}

// ============================================================================
// GL Seed Tests
// ============================================================================

#[tokio::test]
async fn test_seed_gl_creates_accounting_period() {
    let gl_pool = get_gl_pool().await;
    let tenant_id = Uuid::new_v4();

    // Seed GL data for tenant
    seed_gl_module(&gl_pool, tenant_id)
        .await
        .expect("GL seed should succeed");

    // Verify accounting period was created
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounting_periods WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .fetch_one(&gl_pool)
            .await
            .expect("Failed to count accounting periods");

    assert_eq!(count, 1, "Should have exactly one accounting period");

    // Verify period is not closed
    let is_closed: bool =
        sqlx::query_scalar("SELECT is_closed FROM accounting_periods WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .fetch_one(&gl_pool)
            .await
            .expect("Failed to fetch is_closed");

    assert!(!is_closed, "New accounting period should not be closed");

    // Cleanup
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id.to_string())
        .execute(&gl_pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_seed_gl_is_idempotent() {
    let gl_pool = get_gl_pool().await;
    let tenant_id = Uuid::new_v4();

    // Seed twice — should not error or create duplicates
    seed_gl_module(&gl_pool, tenant_id)
        .await
        .expect("First GL seed should succeed");
    seed_gl_module(&gl_pool, tenant_id)
        .await
        .expect("Second GL seed should succeed (idempotent)");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounting_periods WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .fetch_one(&gl_pool)
            .await
            .expect("Failed to count accounting periods");

    assert_eq!(count, 1, "Idempotent seed should not create duplicates");

    // Cleanup
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id.to_string())
        .execute(&gl_pool)
        .await
        .ok();
}

// ============================================================================
// AR Seed Tests
// ============================================================================

#[tokio::test]
async fn test_seed_ar_creates_dunning_config() {
    let ar_pool = get_ar_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();

    seed_ar_module(&ar_pool, tenant_id)
        .await
        .expect("AR seed should succeed");

    let (grace_days, max_retries): (i32, i32) = sqlx::query_as(
        "SELECT grace_period_days, max_retry_attempts FROM ar_dunning_config WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&ar_pool)
    .await
    .expect("Failed to fetch dunning config");

    assert_eq!(grace_days, 3, "Default grace period should be 3 days");
    assert_eq!(max_retries, 3, "Default max retry attempts should be 3");

    // Cleanup
    sqlx::query("DELETE FROM ar_dunning_config WHERE app_id = $1")
        .bind(&app_id)
        .execute(&ar_pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_seed_ar_is_idempotent() {
    let ar_pool = get_ar_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();

    seed_ar_module(&ar_pool, tenant_id)
        .await
        .expect("First AR seed should succeed");
    seed_ar_module(&ar_pool, tenant_id)
        .await
        .expect("Second AR seed should succeed (idempotent)");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ar_dunning_config WHERE app_id = $1")
        .bind(&app_id)
        .fetch_one(&ar_pool)
        .await
        .expect("Failed to count dunning configs");

    assert_eq!(count, 1, "Idempotent seed should not create duplicates");

    // Cleanup
    sqlx::query("DELETE FROM ar_dunning_config WHERE app_id = $1")
        .bind(&app_id)
        .execute(&ar_pool)
        .await
        .ok();
}

// ============================================================================
// Subscriptions Seed Tests
// ============================================================================

#[tokio::test]
async fn test_seed_subscriptions_creates_default_plan() {
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = Uuid::new_v4();
    let tenant_id_str = tenant_id.to_string();

    seed_subscriptions_module(&subs_pool, tenant_id)
        .await
        .expect("Subscriptions seed should succeed");

    let (name, schedule, price_minor, currency): (String, String, i64, String) = sqlx::query_as(
        r#"
            SELECT name, schedule, price_minor, currency
            FROM subscription_plans
            WHERE tenant_id = $1 AND name = 'Standard Monthly'
            "#,
    )
    .bind(&tenant_id_str)
    .fetch_one(&subs_pool)
    .await
    .expect("Failed to fetch subscription plan");

    assert_eq!(name, "Standard Monthly");
    assert_eq!(schedule, "monthly");
    assert_eq!(price_minor, 9900);
    assert_eq!(currency, "usd");

    // Cleanup
    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(&tenant_id_str)
        .execute(&subs_pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_seed_subscriptions_is_idempotent() {
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = Uuid::new_v4();
    let tenant_id_str = tenant_id.to_string();

    seed_subscriptions_module(&subs_pool, tenant_id)
        .await
        .expect("First subscriptions seed should succeed");
    seed_subscriptions_module(&subs_pool, tenant_id)
        .await
        .expect("Second subscriptions seed should succeed (idempotent)");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_plans WHERE tenant_id = $1 AND name = 'Standard Monthly'",
    )
    .bind(&tenant_id_str)
    .fetch_one(&subs_pool)
    .await
    .expect("Failed to count plans");

    assert_eq!(
        count, 1,
        "Idempotent seed should not create duplicate plans"
    );

    // Cleanup
    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(&tenant_id_str)
        .execute(&subs_pool)
        .await
        .ok();
}

// ============================================================================
// Identity Seed Tests
// ============================================================================

#[tokio::test]
async fn test_seed_identity_creates_admin_user() {
    std::env::set_var("SEED_ADMIN_PASSWORD", "TestSeedPw@Integration1!");
    let auth_pool = get_auth_pool().await;
    let tenant_id = Uuid::new_v4();
    let expected_email = format!("admin@{}.local", tenant_id);

    seed_identity_module(&auth_pool, tenant_id)
        .await
        .expect("Identity seed should succeed");

    let (email, is_active): (String, bool) =
        sqlx::query_as("SELECT email, is_active FROM credentials WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&auth_pool)
            .await
            .expect("Failed to fetch admin credential");

    assert_eq!(email, expected_email);
    assert!(is_active, "Admin user should be active");

    // Cleanup
    sqlx::query("DELETE FROM credentials WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&auth_pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_seed_identity_is_idempotent() {
    std::env::set_var("SEED_ADMIN_PASSWORD", "TestSeedPw@Integration1!");
    let auth_pool = get_auth_pool().await;
    let tenant_id = Uuid::new_v4();

    seed_identity_module(&auth_pool, tenant_id)
        .await
        .expect("First identity seed should succeed");
    seed_identity_module(&auth_pool, tenant_id)
        .await
        .expect("Second identity seed should succeed (idempotent)");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credentials WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&auth_pool)
        .await
        .expect("Failed to count credentials");

    assert_eq!(
        count, 1,
        "Idempotent seed should not create duplicate users"
    );

    // Cleanup
    sqlx::query("DELETE FROM credentials WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&auth_pool)
        .await
        .ok();
}

// ============================================================================
// Activation Tests
// ============================================================================

#[tokio::test]
async fn test_activate_tenant_sets_status_active() {
    let registry_pool = get_tenant_registry_pool().await;
    let tenant_id = Uuid::new_v4();

    insert_provisioning_tenant(&registry_pool, tenant_id).await;

    activate_tenant_atomic(&registry_pool, tenant_id)
        .await
        .expect("Activation should succeed");

    let status = get_tenant_status(&registry_pool, tenant_id).await;
    assert_eq!(
        status.as_deref(),
        Some("active"),
        "Tenant should be active after activation"
    );

    cleanup_tenant(&registry_pool, tenant_id).await;
}

#[tokio::test]
async fn test_activate_tenant_emits_provisioned_outbox_event() {
    let registry_pool = get_tenant_registry_pool().await;
    let tenant_id = Uuid::new_v4();

    insert_provisioning_tenant(&registry_pool, tenant_id).await;

    activate_tenant_atomic(&registry_pool, tenant_id)
        .await
        .expect("Activation should succeed");

    let event_count = get_outbox_event_count(&registry_pool, tenant_id, "tenant.provisioned").await;
    assert_eq!(
        event_count, 1,
        "Should have exactly one tenant.provisioned outbox event"
    );

    cleanup_tenant(&registry_pool, tenant_id).await;
}

#[tokio::test]
async fn test_activate_tenant_is_atomic_status_and_outbox() {
    // Verify that status=active AND outbox event exist together — no partial state
    let registry_pool = get_tenant_registry_pool().await;
    let tenant_id = Uuid::new_v4();

    insert_provisioning_tenant(&registry_pool, tenant_id).await;

    activate_tenant_atomic(&registry_pool, tenant_id)
        .await
        .expect("Activation should succeed");

    // Check both in one query to prove atomicity
    let (status, outbox_count): (String, i64) = sqlx::query_as(
        r#"
        SELECT t.status, COUNT(o.id)
        FROM tenants t
        LEFT JOIN provisioning_outbox o
            ON o.tenant_id = t.tenant_id
            AND o.event_type = 'tenant.provisioned'
        WHERE t.tenant_id = $1
        GROUP BY t.status
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&registry_pool)
    .await
    .expect("Failed to fetch activation state");

    assert_eq!(status, "active", "Status must be active");
    assert_eq!(outbox_count, 1, "Must have exactly one provisioned event");

    cleanup_tenant(&registry_pool, tenant_id).await;
}

#[tokio::test]
async fn test_activate_nonexistent_tenant_returns_error() {
    let registry_pool = get_tenant_registry_pool().await;
    let fake_tenant_id = Uuid::new_v4(); // never inserted

    let result = activate_tenant_atomic(&registry_pool, fake_tenant_id).await;
    assert!(result.is_err(), "Should fail for non-existent tenant");
}

#[tokio::test]
async fn test_activate_already_active_tenant_returns_error() {
    let registry_pool = get_tenant_registry_pool().await;
    let tenant_id = Uuid::new_v4();

    // Insert as already active (not provisioning)
    sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions) VALUES ($1, 'active', 'development', '{}'::jsonb)",
    )
    .bind(tenant_id)
    .execute(&registry_pool)
    .await
    .expect("Failed to insert active tenant");

    // Activation should fail because guard requires status='provisioning'
    let result = activate_tenant_atomic(&registry_pool, tenant_id).await;
    assert!(result.is_err(), "Should fail when tenant is already active");

    cleanup_tenant(&registry_pool, tenant_id).await;
}

// ============================================================================
// Health Check Tests
// ============================================================================

#[tokio::test]
async fn test_health_check_empty_modules_is_ready() {
    let client = reqwest::Client::new();
    let result = check_all_modules_ready(&client, &[]).await;
    assert!(
        result.all_ready,
        "Empty module list should be vacuously ready"
    );
}

#[tokio::test]
async fn test_health_check_unavailable_module_blocks_activation() {
    let registry_pool = get_tenant_registry_pool().await;
    let tenant_id = Uuid::new_v4();

    insert_provisioning_tenant(&registry_pool, tenant_id).await;

    // Point to a port that won't respond
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(200))
        .build()
        .unwrap();
    let module_urls = vec![ModuleUrl::new("fake-module", "http://127.0.0.1:19998")];
    let health = check_all_modules_ready(&client, &module_urls).await;

    assert!(
        !health.all_ready,
        "Health check should fail for unreachable module"
    );

    // Tenant should still be in provisioning state (activation was not called)
    let status = get_tenant_status(&registry_pool, tenant_id).await;
    assert_eq!(
        status.as_deref(),
        Some("provisioning"),
        "Tenant should remain in provisioning state when health check fails"
    );

    cleanup_tenant(&registry_pool, tenant_id).await;
}
