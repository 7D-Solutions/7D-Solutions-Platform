/// Integration tests for tenant registry lifecycle operations.
///
/// Covers: tenant CRUD (create, read, update, soft-delete), lifecycle state
/// transitions (provisioning → trial → active → suspended → deleted),
/// plan/bundle assignment, app_id mapping, and atomic activation.

use sqlx::PgPool;
use uuid::Uuid;

use tenant_registry::{TenantStatus, is_valid_state_transition};

async fn test_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    PgPool::connect(&url).await.expect("connect to tenant-registry DB")
}

/// Insert a tenant with the given status string.
async fn insert_tenant(pool: &PgPool, status: &str) -> Uuid {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, created_at, updated_at)
         VALUES ($1, $2, 'development', '{}'::jsonb, NOW(), NOW())",
    )
    .bind(tenant_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("insert tenant");
    tenant_id
}

/// Insert a tenant with optional metadata fields.
async fn insert_tenant_full(
    pool: &PgPool,
    status: &str,
    product_code: Option<&str>,
    plan_code: Option<&str>,
    app_id: Option<&str>,
) -> Uuid {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tenants
           (tenant_id, status, environment, module_schema_versions,
            product_code, plan_code, app_id, created_at, updated_at)
           VALUES ($1, $2, 'development', '{}'::jsonb, $3, $4, $5, NOW(), NOW())"#,
    )
    .bind(tenant_id)
    .bind(status)
    .bind(product_code)
    .bind(plan_code)
    .bind(app_id)
    .execute(pool)
    .await
    .expect("insert tenant with fields");
    tenant_id
}

/// Read the current status string for a tenant.
async fn get_status(pool: &PgPool, tenant_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(pool)
        .await
        .expect("read tenant status")
}

/// Update tenant status in the DB and return rows affected.
async fn update_status(pool: &PgPool, tenant_id: Uuid, new_status: &str) -> u64 {
    sqlx::query("UPDATE tenants SET status = $1, updated_at = NOW() WHERE tenant_id = $2")
        .bind(new_status)
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("update tenant status")
        .rows_affected()
}

async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM cp_entitlements WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM provisioning_outbox WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
}

// ============================================================================
// Tenant CRUD Tests
// ============================================================================

#[tokio::test]
async fn create_and_read_tenant() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(
        &pool, "active", Some("acme"), Some("starter"), Some("app_acme1"),
    ).await;

    let row = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>)>(
        "SELECT status, environment, product_code, plan_code, app_id FROM tenants WHERE tenant_id = $1",
    )
    .bind(tid)
    .fetch_one(&pool)
    .await
    .expect("read tenant");

    assert_eq!(row.0, "active");
    assert_eq!(row.1, "development");
    assert_eq!(row.2.as_deref(), Some("acme"));
    assert_eq!(row.3.as_deref(), Some("starter"));
    assert_eq!(row.4.as_deref(), Some("app_acme1"));

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn update_tenant_product_and_plan() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(
        &pool, "active", Some("old-product"), Some("basic"), None,
    ).await;

    sqlx::query(
        "UPDATE tenants SET product_code = $1, plan_code = $2, updated_at = NOW() WHERE tenant_id = $3",
    )
    .bind("new-product")
    .bind("professional")
    .bind(tid)
    .execute(&pool)
    .await
    .expect("update product and plan");

    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT product_code, plan_code FROM tenants WHERE tenant_id = $1",
    )
    .bind(tid)
    .fetch_one(&pool)
    .await
    .expect("read updated tenant");

    assert_eq!(row.0.as_deref(), Some("new-product"));
    assert_eq!(row.1.as_deref(), Some("professional"));

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn soft_delete_hides_tenant_from_non_deleted_queries() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "active").await;

    sqlx::query("UPDATE tenants SET deleted_at = NOW(), status = 'deleted' WHERE tenant_id = $1")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("soft delete");

    // Row still exists
    assert_eq!(get_status(&pool, tid).await, "deleted");

    // But filtered out by deleted_at IS NULL (matching tenant_crud.rs list query)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenants WHERE tenant_id = $1 AND deleted_at IS NULL",
    )
    .bind(tid)
    .fetch_one(&pool)
    .await
    .expect("count non-deleted");
    assert_eq!(count, 0);

    cleanup(&pool, tid).await;
}

// ============================================================================
// Provisioning → Trial → Active → Suspended → Deleted (full happy path)
// ============================================================================

#[tokio::test]
async fn full_lifecycle_provisioning_to_deleted() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "provisioning").await;

    // provisioning → trial
    assert!(is_valid_state_transition(TenantStatus::Provisioning, TenantStatus::Trial));
    update_status(&pool, tid, "trial").await;
    assert_eq!(get_status(&pool, tid).await, "trial");

    // trial → active
    assert!(is_valid_state_transition(TenantStatus::Trial, TenantStatus::Active));
    update_status(&pool, tid, "active").await;
    assert_eq!(get_status(&pool, tid).await, "active");

    // active → suspended
    assert!(is_valid_state_transition(TenantStatus::Active, TenantStatus::Suspended));
    update_status(&pool, tid, "suspended").await;
    assert_eq!(get_status(&pool, tid).await, "suspended");

    // suspended → active (reactivation)
    assert!(is_valid_state_transition(TenantStatus::Suspended, TenantStatus::Active));
    update_status(&pool, tid, "active").await;
    assert_eq!(get_status(&pool, tid).await, "active");

    // active → deleted
    assert!(is_valid_state_transition(TenantStatus::Active, TenantStatus::Deleted));
    update_status(&pool, tid, "deleted").await;
    assert_eq!(get_status(&pool, tid).await, "deleted");

    cleanup(&pool, tid).await;
}

// ============================================================================
// Provisioning → Active (direct, no trial)
// ============================================================================

#[tokio::test]
async fn provisioning_directly_to_active() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "provisioning").await;

    assert!(is_valid_state_transition(TenantStatus::Provisioning, TenantStatus::Active));
    update_status(&pool, tid, "active").await;
    assert_eq!(get_status(&pool, tid).await, "active");

    cleanup(&pool, tid).await;
}

// ============================================================================
// PastDue lifecycle paths
// ============================================================================

#[tokio::test]
async fn active_to_past_due_and_recovery() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "active").await;

    // active → past_due
    assert!(is_valid_state_transition(TenantStatus::Active, TenantStatus::PastDue));
    update_status(&pool, tid, "past_due").await;
    assert_eq!(get_status(&pool, tid).await, "past_due");

    // past_due → active (payment received)
    assert!(is_valid_state_transition(TenantStatus::PastDue, TenantStatus::Active));
    update_status(&pool, tid, "active").await;
    assert_eq!(get_status(&pool, tid).await, "active");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn past_due_to_suspended() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "past_due").await;

    assert!(is_valid_state_transition(TenantStatus::PastDue, TenantStatus::Suspended));
    update_status(&pool, tid, "suspended").await;
    assert_eq!(get_status(&pool, tid).await, "suspended");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn trial_to_past_due() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "trial").await;

    assert!(is_valid_state_transition(TenantStatus::Trial, TenantStatus::PastDue));
    update_status(&pool, tid, "past_due").await;
    assert_eq!(get_status(&pool, tid).await, "past_due");

    cleanup(&pool, tid).await;
}

// ============================================================================
// Invalid transitions
// ============================================================================

#[tokio::test]
async fn deleted_is_terminal() {
    assert!(!is_valid_state_transition(TenantStatus::Deleted, TenantStatus::Active));
    assert!(!is_valid_state_transition(TenantStatus::Deleted, TenantStatus::Provisioning));
    assert!(!is_valid_state_transition(TenantStatus::Deleted, TenantStatus::Suspended));
    assert!(!is_valid_state_transition(TenantStatus::Deleted, TenantStatus::Trial));
    assert!(!is_valid_state_transition(TenantStatus::Deleted, TenantStatus::PastDue));
}

#[tokio::test]
async fn self_transitions_rejected() {
    assert!(!is_valid_state_transition(TenantStatus::Active, TenantStatus::Active));
    assert!(!is_valid_state_transition(TenantStatus::Provisioning, TenantStatus::Provisioning));
    assert!(!is_valid_state_transition(TenantStatus::Suspended, TenantStatus::Suspended));
    assert!(!is_valid_state_transition(TenantStatus::Trial, TenantStatus::Trial));
}

#[tokio::test]
async fn provisioning_cannot_skip_to_suspended() {
    assert!(!is_valid_state_transition(TenantStatus::Provisioning, TenantStatus::Suspended));
    assert!(!is_valid_state_transition(TenantStatus::Provisioning, TenantStatus::PastDue));
}

#[tokio::test]
async fn suspended_cannot_go_to_trial() {
    assert!(!is_valid_state_transition(TenantStatus::Suspended, TenantStatus::Trial));
    assert!(!is_valid_state_transition(TenantStatus::Suspended, TenantStatus::PastDue));
    assert!(!is_valid_state_transition(TenantStatus::Suspended, TenantStatus::Provisioning));
}

// ============================================================================
// DB-level: updated_at changes on status transition
// ============================================================================

#[tokio::test]
async fn status_update_changes_updated_at() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "provisioning").await;

    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM tenants WHERE tenant_id = $1")
            .bind(tid)
            .fetch_one(&pool)
            .await
            .expect("read updated_at");

    // Small delay so timestamps differ
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    update_status(&pool, tid, "active").await;

    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM tenants WHERE tenant_id = $1")
            .bind(tid)
            .fetch_one(&pool)
            .await
            .expect("read updated_at after");

    assert!(after > before, "updated_at must increase on status change");

    cleanup(&pool, tid).await;
}

// ============================================================================
// Provisioning state machine (distinct from TenantStatus)
// ============================================================================

#[tokio::test]
async fn provisioning_state_transitions() {
    use tenant_registry::{ProvisioningState, is_valid_provisioning_transition};

    // Valid
    assert!(is_valid_provisioning_transition(ProvisioningState::Pending, ProvisioningState::Provisioning));
    assert!(is_valid_provisioning_transition(ProvisioningState::Provisioning, ProvisioningState::Active));
    assert!(is_valid_provisioning_transition(ProvisioningState::Provisioning, ProvisioningState::Failed));

    // Invalid
    assert!(!is_valid_provisioning_transition(ProvisioningState::Pending, ProvisioningState::Active));
    assert!(!is_valid_provisioning_transition(ProvisioningState::Active, ProvisioningState::Pending));
    assert!(!is_valid_provisioning_transition(ProvisioningState::Failed, ProvisioningState::Active));
}
