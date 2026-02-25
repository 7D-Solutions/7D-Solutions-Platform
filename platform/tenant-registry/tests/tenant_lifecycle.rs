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

// ============================================================================
// Plan / Bundle Assignment Tests
// ============================================================================

#[tokio::test]
async fn assign_plan_to_tenant() {
    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "active").await;

    sqlx::query("UPDATE tenants SET plan_code = $1, updated_at = NOW() WHERE tenant_id = $2")
        .bind("professional")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("assign plan");

    let plan: Option<String> =
        sqlx::query_scalar("SELECT plan_code FROM tenants WHERE tenant_id = $1")
            .bind(tid)
            .fetch_one(&pool)
            .await
            .expect("read plan");
    assert_eq!(plan.as_deref(), Some("professional"));

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn bundle_assignment_and_transition() {
    let pool = test_pool().await;

    // Ensure bundle tables exist
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS cp_bundles (
            bundle_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            product_code TEXT NOT NULL,
            bundle_name TEXT NOT NULL,
            is_default BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
    )
    .execute(&pool)
    .await
    .expect("ensure cp_bundles");

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS cp_tenant_bundle (
            tenant_id UUID NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
            bundle_id UUID NOT NULL REFERENCES cp_bundles(bundle_id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'active',
            effective_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (tenant_id, bundle_id)
        )"#,
    )
    .execute(&pool)
    .await
    .expect("ensure cp_tenant_bundle");

    let tid = insert_tenant(&pool, "active").await;

    // Create a bundle
    let bundle_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO cp_bundles (product_code, bundle_name, is_default)
           VALUES ('starter', 'Starter Bundle', true)
           RETURNING bundle_id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("create bundle");

    // Assign bundle to tenant
    sqlx::query(
        "INSERT INTO cp_tenant_bundle (tenant_id, bundle_id, status) VALUES ($1, $2, 'active')",
    )
    .bind(tid)
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("assign bundle");

    // Verify assignment
    let status: String = sqlx::query_scalar(
        "SELECT status FROM cp_tenant_bundle WHERE tenant_id = $1 AND bundle_id = $2",
    )
    .bind(tid)
    .bind(bundle_id)
    .fetch_one(&pool)
    .await
    .expect("read bundle status");
    assert_eq!(status, "active");

    // Transition to in_transition (simulating upgrade)
    sqlx::query(
        "UPDATE cp_tenant_bundle SET status = 'in_transition' WHERE tenant_id = $1 AND bundle_id = $2",
    )
    .bind(tid)
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("transition bundle");

    let status: String = sqlx::query_scalar(
        "SELECT status FROM cp_tenant_bundle WHERE tenant_id = $1 AND bundle_id = $2",
    )
    .bind(tid)
    .bind(bundle_id)
    .fetch_one(&pool)
    .await
    .expect("read transitioned status");
    assert_eq!(status, "in_transition");

    // Cleanup
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tid).execute(&pool).await.ok();
    sqlx::query("DELETE FROM cp_bundles WHERE bundle_id = $1")
        .bind(bundle_id).execute(&pool).await.ok();
    cleanup(&pool, tid).await;
}

// ============================================================================
// App-ID Mapping Tests
// ============================================================================

#[tokio::test]
async fn app_id_stored_and_retrieved() {
    use tenant_registry::get_tenant_app_id;

    let pool = test_pool().await;
    let app_id = format!("app_{}", &Uuid::new_v4().to_string()[..8]);
    let tid = insert_tenant_full(&pool, "active", Some("starter"), None, Some(&app_id)).await;

    let result = get_tenant_app_id(&pool, tid).await.expect("get app_id");
    assert!(result.is_some());
    let row = result.unwrap();
    assert_eq!(row.app_id, app_id);
    assert_eq!(row.product_code.as_deref(), Some("starter"));

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn app_id_none_when_null() {
    use tenant_registry::get_tenant_app_id;

    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "active").await;

    let result = get_tenant_app_id(&pool, tid).await.expect("get app_id");
    assert!(result.is_none(), "app_id should be None when NULL in DB");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn app_id_error_for_missing_tenant() {
    use tenant_registry::get_tenant_app_id;

    let pool = test_pool().await;
    let result = get_tenant_app_id(&pool, Uuid::new_v4()).await;
    assert!(result.is_err(), "should error for nonexistent tenant");
}

// ============================================================================
// Entitlements Tests
// ============================================================================

#[tokio::test]
async fn entitlements_returned_for_tenant() {
    use tenant_registry::get_tenant_entitlements;

    let pool = test_pool().await;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS cp_entitlements (
            tenant_id UUID PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,
            plan_code TEXT NOT NULL,
            concurrent_user_limit INT NOT NULL CHECK (concurrent_user_limit > 0),
            effective_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )"#,
    )
    .execute(&pool)
    .await
    .expect("ensure cp_entitlements");

    let tid = insert_tenant(&pool, "active").await;

    sqlx::query(
        "INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit) VALUES ($1, 'enterprise', 50)",
    )
    .bind(tid)
    .execute(&pool)
    .await
    .expect("insert entitlements");

    let result = get_tenant_entitlements(&pool, tid).await.expect("get entitlements");
    assert!(result.is_some());
    let ent = result.unwrap();
    assert_eq!(ent.plan_code, "enterprise");
    assert_eq!(ent.concurrent_user_limit, 50);

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn entitlements_none_when_no_row() {
    use tenant_registry::get_tenant_entitlements;

    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "active").await;

    let result = get_tenant_entitlements(&pool, tid).await.expect("get entitlements");
    assert!(result.is_none());

    cleanup(&pool, tid).await;
}

// ============================================================================
// Tenant Status Row (lightweight endpoint backing)
// ============================================================================

#[tokio::test]
async fn tenant_status_row_returned() {
    use tenant_registry::get_tenant_status_row;

    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "trial").await;

    let result = get_tenant_status_row(&pool, tid).await.expect("get status row");
    assert!(result.is_some());
    assert_eq!(result.unwrap().status, "trial");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn tenant_status_row_none_for_missing() {
    use tenant_registry::get_tenant_status_row;

    let pool = test_pool().await;
    let result = get_tenant_status_row(&pool, Uuid::new_v4())
        .await
        .expect("get status row");
    assert!(result.is_none());
}

// ============================================================================
// Atomic Activation Tests
// ============================================================================

#[tokio::test]
async fn activate_tenant_atomic_transitions_to_active() {
    use tenant_registry::activate_tenant_atomic;

    let pool = test_pool().await;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS provisioning_outbox (
            id BIGSERIAL PRIMARY KEY,
            tenant_id UUID NOT NULL,
            event_type TEXT NOT NULL,
            payload JSONB NOT NULL DEFAULT '{}'::jsonb,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            published_at TIMESTAMPTZ
        )"#,
    )
    .execute(&pool)
    .await
    .expect("ensure provisioning_outbox");

    let tid = insert_tenant(&pool, "provisioning").await;

    activate_tenant_atomic(&pool, tid).await.expect("activate tenant");

    assert_eq!(get_status(&pool, tid).await, "active");

    // Verify outbox event
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM provisioning_outbox WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tid)
    .fetch_one(&pool)
    .await
    .expect("read outbox event");
    assert_eq!(event_type, "tenant.provisioned");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn activate_tenant_atomic_fails_if_not_provisioning() {
    use tenant_registry::activate_tenant_atomic;

    let pool = test_pool().await;
    let tid = insert_tenant(&pool, "active").await;

    let result = activate_tenant_atomic(&pool, tid).await;
    assert!(result.is_err(), "should fail when not in provisioning state");

    cleanup(&pool, tid).await;
}
