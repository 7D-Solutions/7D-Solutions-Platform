//! Integration tests for GET /api/control/tenants/{tenant_id}/vitals (bd-yd6r2).
//!
//! All tests use real Postgres (TENANT_REGISTRY_DATABASE_URL).
//! Module HTTP calls use real axum test servers bound to local ports.
//! No mocks. No stubs.

use axum::{routing::get, Json, Router};
use axum_test::TestServer;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use control_plane::routes::build_router;
use control_plane::state::AppState;
use tenant_registry::routes::SummaryState;

// ============================================================================
// Helpers
// ============================================================================

async fn test_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    sqlx::pool::PoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

fn build_server(pool: PgPool) -> TestServer {
    let app_state = Arc::new(AppState::new(pool.clone(), None));
    let summary_state = Arc::new(SummaryState::new_local(pool));
    let router = build_router(app_state, summary_state);
    TestServer::new(router).expect("build test server")
}

/// Seed a minimal tenant row and provisioning steps, returning the tenant_id.
async fn seed_tenant(pool: &PgPool, steps: &[(&str, i32, &str)]) -> Uuid {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants (tenant_id, idempotency_key, status, environment, product_code, plan_code, app_id, concurrent_user_limit) \
         VALUES ($1, $2, 'active', 'development', 'starter', 'monthly', $3, 5)",
    )
    .bind(tenant_id)
    .bind(Uuid::new_v4().to_string())
    .bind(format!("app-{}", &tenant_id.to_string()[..12]))
    .execute(pool)
    .await
    .expect("seed tenant");

    for (name, order, status) in steps {
        sqlx::query(
            "INSERT INTO provisioning_steps (tenant_id, step_name, step_order, status) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant_id)
        .bind(name)
        .bind(order)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed provisioning step");
    }

    tenant_id
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM provisioning_steps WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_tenant_module_status WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

fn healthy_vitals_response() -> health::VitalsResponse {
    health::VitalsResponse {
        service_name: "test-module".to_string(),
        version: "1.0.0".to_string(),
        tenant_ready: Some(true),
        dlq: health::DlqVitals {
            total: 0,
            retryable: 0,
            fatal: 0,
            poison: 0,
        },
        outbox: health::OutboxVitals {
            pending: 0,
            oldest_pending_secs: None,
        },
        projections: vec![],
        consumers: vec![],
        extended: None,
        timestamp: Utc::now().to_rfc3339(),
    }
}

fn unhealthy_vitals_response(dlq_total: u64) -> health::VitalsResponse {
    health::VitalsResponse {
        service_name: "test-module".to_string(),
        version: "1.0.0".to_string(),
        tenant_ready: Some(true),
        dlq: health::DlqVitals {
            total: dlq_total,
            retryable: 0,
            fatal: dlq_total,
            poison: 0,
        },
        outbox: health::OutboxVitals {
            pending: 0,
            oldest_pending_secs: None,
        },
        projections: vec![],
        consumers: vec![],
        extended: None,
        timestamp: Utc::now().to_rfc3339(),
    }
}

/// Spawn an axum test server on a random OS-assigned port and return (url, server).
/// The server is kept alive for the duration of the test via the returned handle.
async fn healthy_vitals_handler() -> Json<health::VitalsResponse> {
    Json(healthy_vitals_response())
}

async fn slow_vitals_handler() -> Json<health::VitalsResponse> {
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    Json(healthy_vitals_response())
}

async fn dlq_vitals_handler() -> Json<health::VitalsResponse> {
    Json(unhealthy_vitals_response(1))
}

async fn spawn_vitals_server_healthy() -> (String, axum_test::TestServer) {
    spawn_vitals_server_with(healthy_vitals_handler).await
}

async fn spawn_vitals_server_slow() -> (String, axum_test::TestServer) {
    spawn_vitals_server_with(slow_vitals_handler).await
}

async fn spawn_vitals_server_dlq() -> (String, axum_test::TestServer) {
    spawn_vitals_server_with(dlq_vitals_handler).await
}

async fn spawn_vitals_server_with<H, T>(handler: H) -> (String, axum_test::TestServer)
where
    H: axum::handler::Handler<T, ()> + Clone + Send + 'static,
    T: Send + 'static,
{
    let router = Router::new().route("/api/vitals", get(handler));
    let server = axum_test::TestServer::builder()
        .http_transport()
        .build(router)
        .expect("build vitals test server");
    let addr = server.server_address().expect("get server address");
    let url = format!("http://{}", addr);
    (url, server)
}

// ============================================================================
// Tests
// ============================================================================

/// tenant_vitals_all_steps_complete_no_modules:
/// Tenant with all steps completed, no bundle rows → overall_healthy=true, modules=[]
#[tokio::test]
async fn tenant_vitals_all_steps_complete_no_modules() {
    let pool = test_pool().await;
    let tenant_id = seed_tenant(
        &pool,
        &[
            ("create_schema", 1, "completed"),
            ("seed_data", 2, "completed"),
        ],
    )
    .await;

    let server = build_server(pool.clone());
    let resp = server
        .get(&format!("/api/control/tenants/{tenant_id}/vitals"))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert_eq!(body["overall_healthy"], true);
    assert_eq!(body["provisioning"]["all_steps_complete"], true);
    assert_eq!(
        body["modules"].as_array().unwrap().len(),
        0,
        "no modules expected"
    );

    cleanup_tenant(&pool, tenant_id).await;
}

/// tenant_vitals_pending_step_not_healthy:
/// Tenant with one step still pending → overall_healthy=false
#[tokio::test]
async fn tenant_vitals_pending_step_not_healthy() {
    let pool = test_pool().await;
    let tenant_id = seed_tenant(
        &pool,
        &[
            ("create_schema", 1, "completed"),
            ("seed_data", 2, "pending"),
        ],
    )
    .await;

    let server = build_server(pool.clone());
    let resp = server
        .get(&format!("/api/control/tenants/{tenant_id}/vitals"))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["overall_healthy"], false);
    assert_eq!(body["provisioning"]["all_steps_complete"], false);

    cleanup_tenant(&pool, tenant_id).await;
}

/// tenant_vitals_module_slow_excluded:
/// Module server sleeps 3s; 2s per-module timeout fires; error is Some; overall_healthy
/// is NOT flipped false by this alone (steps are all complete, no other modules).
#[tokio::test]
async fn tenant_vitals_module_slow_excluded() {
    let pool = test_pool().await;

    // Spawn a slow vitals server (responds after 3s)
    let (base_url, _server) = spawn_vitals_server_slow().await;

    // Register a fake module in the service catalog
    let module_code = format!("slow_{}", &Uuid::new_v4().to_string()[..8]);
    sqlx::query(
        "INSERT INTO cp_service_catalog (module_code, base_url) VALUES ($1, $2) \
         ON CONFLICT (module_code) DO UPDATE SET base_url = EXCLUDED.base_url",
    )
    .bind(&module_code)
    .bind(&base_url)
    .execute(&pool)
    .await
    .expect("seed catalog");

    // Seed tenant with completed steps and a bundle pointing at this module
    let tenant_id = seed_tenant(&pool, &[("create_schema", 1, "completed")]).await;

    // Create a bundle and attach the module
    let bundle_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cp_bundles (bundle_id, bundle_name, product_code, is_default) \
         VALUES ($1, $2, 'starter', false)",
    )
    .bind(bundle_id)
    .bind(format!("test_bundle_{}", &bundle_id.to_string()[..8]))
    .execute(&pool)
    .await
    .expect("seed bundle");

    sqlx::query(
        "INSERT INTO cp_bundle_modules (bundle_id, module_code) VALUES ($1, $2)",
    )
    .bind(bundle_id)
    .bind(&module_code)
    .execute(&pool)
    .await
    .expect("seed bundle_modules");

    sqlx::query(
        "INSERT INTO cp_tenant_bundle (tenant_id, bundle_id) VALUES ($1, $2)",
    )
    .bind(tenant_id)
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("seed tenant_bundle");

    let server = build_server(pool.clone());
    let resp = server
        .get(&format!("/api/control/tenants/{tenant_id}/vitals"))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();

    // Module entry should have an error (timeout)
    let modules = body["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 1);
    let m = &modules[0];
    assert!(
        m["error"].as_str().is_some(),
        "timed-out module must have error set"
    );
    assert!(m["vitals"].is_null(), "timed-out module must have null vitals");

    // overall_healthy not false from this module alone (excluded from gate)
    assert_eq!(
        body["overall_healthy"], true,
        "timed-out module must not flip overall_healthy"
    );

    // Cleanup
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_bundle_modules WHERE bundle_id = $1")
        .bind(bundle_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_bundles WHERE bundle_id = $1")
        .bind(bundle_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_service_catalog WHERE module_code = $1")
        .bind(&module_code)
        .execute(&pool)
        .await
        .ok();
    cleanup_tenant(&pool, tenant_id).await;
}

/// tenant_vitals_module_dlq_not_healthy:
/// Module returns VitalsResponse with dlq.total=1, tenant_ready=true, outbox.pending=0
/// → overall_healthy=false
#[tokio::test]
async fn tenant_vitals_module_dlq_not_healthy() {
    let pool = test_pool().await;

    let (base_url, _server) = spawn_vitals_server_dlq().await;

    let module_code = format!("dlq_{}", &Uuid::new_v4().to_string()[..8]);
    sqlx::query(
        "INSERT INTO cp_service_catalog (module_code, base_url) VALUES ($1, $2) \
         ON CONFLICT (module_code) DO UPDATE SET base_url = EXCLUDED.base_url",
    )
    .bind(&module_code)
    .bind(&base_url)
    .execute(&pool)
    .await
    .expect("seed catalog");

    let tenant_id = seed_tenant(&pool, &[("create_schema", 1, "completed")]).await;

    let bundle_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cp_bundles (bundle_id, bundle_name, product_code, is_default) \
         VALUES ($1, $2, 'starter', false)",
    )
    .bind(bundle_id)
    .bind(format!("test_bundle_{}", &bundle_id.to_string()[..8]))
    .execute(&pool)
    .await
    .expect("seed bundle");

    sqlx::query(
        "INSERT INTO cp_bundle_modules (bundle_id, module_code) VALUES ($1, $2)",
    )
    .bind(bundle_id)
    .bind(&module_code)
    .execute(&pool)
    .await
    .expect("seed bundle_modules");

    sqlx::query(
        "INSERT INTO cp_tenant_bundle (tenant_id, bundle_id) VALUES ($1, $2)",
    )
    .bind(tenant_id)
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("seed tenant_bundle");

    let server = build_server(pool.clone());
    let resp = server
        .get(&format!("/api/control/tenants/{tenant_id}/vitals"))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["overall_healthy"], false,
        "dlq.total=1 must flip overall_healthy to false"
    );

    let modules = body["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 1);
    assert!(modules[0]["vitals"].is_object(), "vitals must be populated");
    assert_eq!(modules[0]["vitals"]["dlq"]["total"], 1);

    // Cleanup
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_bundle_modules WHERE bundle_id = $1")
        .bind(bundle_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_bundles WHERE bundle_id = $1")
        .bind(bundle_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_service_catalog WHERE module_code = $1")
        .bind(&module_code)
        .execute(&pool)
        .await
        .ok();
    cleanup_tenant(&pool, tenant_id).await;
}
