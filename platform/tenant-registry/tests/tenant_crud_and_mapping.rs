/// Integration tests for tenant CRUD, app_id mapping, plan/bundle assignment
///
/// Covers: (1) tenant create/read/update, (2) app_id generation and lookup,
/// (3) plan assignment and entitlements, (4) bundle assignment.
/// All tests run against a real PostgreSQL database — no mocks.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use tenant_registry::{
    get_tenant_app_id, get_tenant_entitlements, get_tenant_status_row,
    tenant_list_router, tenant_detail_router, plans_router,
    derive_name,
};

async fn test_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    PgPool::connect(&url).await.expect("connect to tenant-registry DB")
}

async fn insert_tenant_full(
    pool: &PgPool,
    status: &str,
    product_code: Option<&str>,
    plan_code: Option<&str>,
    app_id: Option<&str>,
) -> Uuid {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, product_code, plan_code, app_id, created_at, updated_at)
         VALUES ($1, $2, 'development', '{}'::jsonb, $3, $4, $5, NOW(), NOW())",
    )
    .bind(tenant_id)
    .bind(status)
    .bind(product_code)
    .bind(plan_code)
    .bind(app_id)
    .execute(pool)
    .await
    .expect("insert tenant");
    tenant_id
}

async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM cp_entitlements WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
}

// ============================================================================
// Tenant CRUD — Create + Read
// ============================================================================

#[tokio::test]
async fn create_and_read_tenant_via_api() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "active", Some("acme-corp"), Some("starter"), Some("app_acme")).await;

    // Read via detail endpoint
    let app = tenant_detail_router(pool.clone());
    let req = Request::builder()
        .uri(format!("/api/tenants/{tid}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("call detail endpoint");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["id"], tid.to_string());
    assert_eq!(json["status"], "active");
    assert_eq!(json["name"], "acme-corp"); // product_code wins
    assert_eq!(json["plan"], "starter");
    assert!(json["created_at"].is_string());
    assert!(json["updated_at"].is_string());

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn tenant_appears_in_list_endpoint() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "active", Some("list-test"), None, None).await;

    let app = tenant_list_router(pool.clone());
    let req = Request::builder()
        .uri(format!("/api/tenants?search={tid}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("call list endpoint");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let tenants = json["tenants"].as_array().unwrap();
    let found = tenants.iter().any(|t| t["id"].as_str() == Some(&tid.to_string()));
    assert!(found, "tenant must appear in list");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn list_filters_by_status() {
    let pool = test_pool().await;
    let tid_active = insert_tenant_full(&pool, "active", Some("filter-active"), None, None).await;
    let tid_suspended = insert_tenant_full(&pool, "suspended", Some("filter-susp"), None, None).await;

    let app = tenant_list_router(pool.clone());
    let req = Request::builder()
        .uri("/api/tenants?status=suspended&page_size=100")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("call list endpoint");
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let tenants = json["tenants"].as_array().unwrap();
    for t in tenants {
        assert_eq!(t["status"], "suspended", "status filter must work");
    }

    cleanup(&pool, tid_active).await;
    cleanup(&pool, tid_suspended).await;
}

// ============================================================================
// Tenant CRUD — Update (status + plan_code)
// ============================================================================

#[tokio::test]
async fn update_tenant_plan_code() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "active", Some("upgrade-test"), Some("starter"), None).await;

    // Update plan_code
    sqlx::query("UPDATE tenants SET plan_code = $1, updated_at = NOW() WHERE tenant_id = $2")
        .bind("professional")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("update plan_code");

    // Verify via detail endpoint
    let app = tenant_detail_router(pool.clone());
    let req = Request::builder()
        .uri(format!("/api/tenants/{tid}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["plan"], "professional");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn detail_returns_404_for_nonexistent_tenant() {
    let pool = test_pool().await;
    let app = tenant_detail_router(pool);
    let req = Request::builder()
        .uri(format!("/api/tenants/{}", Uuid::new_v4()))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// app_id generation and mapping
// ============================================================================

#[tokio::test]
async fn app_id_lookup_returns_correct_mapping() {
    let pool = test_pool().await;
    let app_id = format!("app_{}", &Uuid::new_v4().to_string()[..8]);
    let tid = insert_tenant_full(&pool, "active", Some("mapping-test"), None, Some(&app_id)).await;

    let result = get_tenant_app_id(&pool, tid).await.expect("lookup app_id");
    assert!(result.is_some());
    let row = result.unwrap();
    assert_eq!(row.app_id, app_id);
    assert_eq!(row.tenant_id, tid);
    assert_eq!(row.product_code.as_deref(), Some("mapping-test"));

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn app_id_null_returns_none() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "active", None, None, None).await;

    let result = get_tenant_app_id(&pool, tid).await.expect("lookup null app_id");
    assert!(result.is_none(), "NULL app_id should return None");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn app_id_nonexistent_tenant_returns_err() {
    let pool = test_pool().await;
    let result = get_tenant_app_id(&pool, Uuid::new_v4()).await;
    assert!(result.is_err(), "nonexistent tenant should return RowNotFound");
}

#[tokio::test]
async fn derive_name_priority_order() {
    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    // product_code wins
    assert_eq!(derive_name(Some("acme"), Some("app_1"), id), "acme");

    // app_id wins when product_code empty
    assert_eq!(derive_name(Some(""), Some("app_1"), id), "app_1");

    // app_id wins when product_code None
    assert_eq!(derive_name(None, Some("app_1"), id), "app_1");

    // falls back to tenant_id prefix
    assert_eq!(derive_name(None, None, id), "550e8400");
    assert_eq!(derive_name(Some(""), Some(""), id), "550e8400");
}

// ============================================================================
// Tenant status endpoint (lightweight lookup)
// ============================================================================

#[tokio::test]
async fn get_status_row_returns_correct_status() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "trial", None, None, None).await;

    let result = get_tenant_status_row(&pool, tid).await.expect("get status");
    assert!(result.is_some());
    assert_eq!(result.unwrap().status, "trial");

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn get_status_row_returns_none_for_missing() {
    let pool = test_pool().await;
    let result = get_tenant_status_row(&pool, Uuid::new_v4()).await.expect("get status");
    assert!(result.is_none());
}

// ============================================================================
// Entitlements
// ============================================================================

#[tokio::test]
async fn entitlements_found_for_tenant_with_row() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "active", None, None, None).await;

    // Ensure table exists
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS cp_entitlements (
            tenant_id UUID PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,
            plan_code TEXT NOT NULL,
            concurrent_user_limit INT NOT NULL CHECK (concurrent_user_limit > 0),
            effective_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )"#,
    )
    .execute(&pool).await.expect("ensure cp_entitlements");

    sqlx::query("INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit) VALUES ($1, 'professional', 50)")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("insert entitlements");

    let result = get_tenant_entitlements(&pool, tid).await.expect("get entitlements");
    assert!(result.is_some());
    let ent = result.unwrap();
    assert_eq!(ent.plan_code, "professional");
    assert_eq!(ent.concurrent_user_limit, 50);

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn entitlements_none_when_no_row() {
    let pool = test_pool().await;
    let tid = insert_tenant_full(&pool, "active", None, None, None).await;

    let result = get_tenant_entitlements(&pool, tid).await.expect("get entitlements");
    assert!(result.is_none());

    cleanup(&pool, tid).await;
}

#[tokio::test]
async fn entitlements_err_for_nonexistent_tenant() {
    let pool = test_pool().await;
    let result = get_tenant_entitlements(&pool, Uuid::new_v4()).await;
    assert!(result.is_err(), "nonexistent tenant should RowNotFound");
}

// ============================================================================
// Plan catalog endpoint
// ============================================================================

#[tokio::test]
async fn plans_endpoint_returns_seeded_plans() {
    let pool = test_pool().await;
    let app = plans_router(pool);

    let req = Request::builder()
        .uri("/api/ttp/plans?page=1&page_size=50")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("call plans endpoint");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let ids: Vec<&str> = json["plans"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();

    assert!(ids.contains(&"starter"), "starter plan must exist");
    assert!(ids.contains(&"professional"), "professional plan must exist");
    assert!(ids.contains(&"enterprise"), "enterprise plan must exist");
}

// ============================================================================
// Bundle assignment (cp_tenant_bundle table)
// ============================================================================

#[tokio::test]
async fn bundle_assignment_persists() {
    let pool = test_pool().await;

    // Ensure cp_bundles and cp_tenant_bundle tables exist
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS cp_bundles (
            bundle_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            product_code TEXT NOT NULL,
            bundle_name TEXT NOT NULL,
            is_default BOOLEAN NOT NULL DEFAULT false,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#,
    )
    .execute(&pool).await.expect("ensure cp_bundles");

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS cp_tenant_bundle (
            tenant_id UUID NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
            bundle_id UUID NOT NULL REFERENCES cp_bundles(bundle_id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'active',
            effective_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (tenant_id, bundle_id)
        )"#,
    )
    .execute(&pool).await.expect("ensure cp_tenant_bundle");

    let tid = insert_tenant_full(&pool, "active", Some("bundle-test"), Some("starter"), None).await;

    // Create a bundle
    let bundle_id: Uuid = sqlx::query_scalar(
        "INSERT INTO cp_bundles (product_code, bundle_name, is_default) VALUES ('starter', 'Starter Bundle', true) RETURNING bundle_id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert bundle");

    // Assign tenant to bundle
    sqlx::query(
        "INSERT INTO cp_tenant_bundle (tenant_id, bundle_id, status) VALUES ($1, $2, 'active')",
    )
    .bind(tid)
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("assign bundle to tenant");

    // Verify assignment
    let assigned_status: String = sqlx::query_scalar(
        "SELECT status FROM cp_tenant_bundle WHERE tenant_id = $1 AND bundle_id = $2",
    )
    .bind(tid)
    .bind(bundle_id)
    .fetch_one(&pool)
    .await
    .expect("read bundle assignment");

    assert_eq!(assigned_status, "active");

    // Cleanup
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1").bind(tid).execute(&pool).await.ok();
    sqlx::query("DELETE FROM cp_bundles WHERE bundle_id = $1").bind(bundle_id).execute(&pool).await.ok();
    cleanup(&pool, tid).await;
}
