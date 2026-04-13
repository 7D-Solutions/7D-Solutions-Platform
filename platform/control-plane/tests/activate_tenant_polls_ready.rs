//! Integration tests: activate_tenant polls per-module /api/ready before
//! marking the tenant active.
//!
//! GAP-16 invariant: a tenant is NEVER set to 'active' until every module in
//! its bundle confirms it is ready to serve that tenant.
//!
//! These tests spin up in-process HTTP servers (real TCP, random port) that
//! simulate module readiness endpoints. No mocks. No stubs. Real Postgres for
//! the tenant-registry database.
//!
//! Required services:
//!   - Tenant registry: TENANT_REGISTRY_DATABASE_URL (default localhost:5441)

use axum::{extract::Query, routing::get, Json, Router};
use control_plane::provisioning::{
    registry::{ModuleProvisioningConfig, ModuleRegistry},
    steps::{activate_tenant, poll_module_readiness},
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn registry_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

/// Apply all control-plane migrations needed by these tests (idempotent).
async fn ensure_tables(pool: &PgPool) {
    let migrations: &[&str] = &[
        include_str!(
            "../../tenant-registry/db/migrations/20260217000001_add_control_plane_tables.sql"
        ),
        include_str!(
            "../../tenant-registry/db/migrations/20260219000001_extend_tenants_product_plan_status_appid.sql"
        ),
        include_str!(
            "../../tenant-registry/db/migrations/20260219000002_add_entitlements.sql"
        ),
        include_str!(
            "../../tenant-registry/db/migrations/20260219000003_add_cp_plans.sql"
        ),
        include_str!(
            "../../tenant-registry/db/migrations/20260220000002_add_bundle_tables.sql"
        ),
        include_str!(
            "../../tenant-registry/db/migrations/20260410000001_add_tenant_module_status.sql"
        ),
    ];
    for sql in migrations {
        sqlx::raw_sql(sql).execute(pool).await.ok();
    }
}

/// Insert a tenant in 'provisioning' status. Returns tenant_id.
async fn insert_provisioning_tenant(pool: &PgPool) -> Uuid {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants \
         (tenant_id, status, environment, module_schema_versions, created_at, updated_at) \
         VALUES ($1, 'provisioning', 'test', '{}'::jsonb, NOW(), NOW()) \
         ON CONFLICT (tenant_id) DO NOTHING",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("insert test tenant");
    tenant_id
}

/// Read tenant status from the DB.
async fn tenant_status(pool: &PgPool, tenant_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .expect("query tenant status")
}

/// Read module statuses from cp_tenant_module_status.
async fn module_statuses(pool: &PgPool, tenant_id: Uuid) -> HashMap<String, String> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT module_code, status FROM cp_tenant_module_status WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .expect("query module statuses");
    rows.into_iter().collect()
}

async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    for table in &[
        "cp_tenant_module_status",
        "provisioning_outbox",
        "provisioning_steps",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE tenant_id = $1"))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// In-process HTTP test server helpers
// ============================================================================

/// State shared between the test and the HTTP handler.
/// `ready` flips to true to make the server respond with `up`.
#[derive(Clone)]
struct ServerState {
    /// When true → respond { tenant: { status: "up" } }.
    /// When false → respond { tenant: { status: "warming" } }.
    ready: Arc<Mutex<bool>>,
    tenant_id_checked: Uuid,
}

#[derive(Deserialize)]
struct ReadyQuery {
    tenant_id: Option<Uuid>,
}

/// Start an in-process HTTP server that serves GET /api/ready?tenant_id=
/// Returns (base_url, addr, ServerState) — the caller controls readiness via state.ready.
async fn start_module_server(initially_ready: bool, tenant_id: Uuid) -> (String, ServerState) {
    let state = ServerState {
        ready: Arc::new(Mutex::new(initially_ready)),
        tenant_id_checked: tenant_id,
    };

    let state_clone = state.clone();

    let app = Router::new().route(
        "/api/ready",
        get(move |Query(q): Query<ReadyQuery>| {
            let s = state_clone.clone();
            async move {
                let is_ready = *s.ready.lock().unwrap();
                let tid = q.tenant_id.unwrap_or(Uuid::nil());
                let tenant_status = if tid == s.tenant_id_checked && is_ready {
                    "up"
                } else {
                    "warming"
                };
                Json(json!({
                    "status": "ready",
                    "tenant": {
                        "id": tid.to_string(),
                        "status": tenant_status
                    }
                }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test server failed");
    });

    (base_url, state)
}

/// Build a ModuleRegistry where each module's http_base_url points to a test server.
fn build_test_registry(modules: Vec<(String, String)>) -> ModuleRegistry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();

    let configs: Vec<(String, ModuleProvisioningConfig)> = modules
        .into_iter()
        .map(|(code, base_url)| {
            let cfg = ModuleProvisioningConfig {
                module_code: code.clone(),
                postgres_host: "localhost".to_string(),
                postgres_port: 5432,
                postgres_user: format!("{code}_user"),
                postgres_password: format!("{code}_pass"),
                migrations_path: root.join(format!("modules/{code}/db/migrations")),
                http_base_url: base_url,
            };
            (code, cfg)
        })
        .collect();

    ModuleRegistry::from_configs(configs)
}

// ============================================================================
// Test 1: all modules respond up immediately → tenant becomes active
// ============================================================================

#[tokio::test]
async fn activate_tenant_polls_ready_all_up_becomes_active() {
    let pool = registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = insert_provisioning_tenant(&pool).await;

    // Start two module servers, both immediately ready
    let (url_a, _state_a) = start_module_server(true, tenant_id).await;
    let (url_b, _state_b) = start_module_server(true, tenant_id).await;

    let module_codes: Vec<String> = vec!["mod-a".to_string(), "mod-b".to_string()];
    let registry = build_test_registry(vec![
        ("mod-a".to_string(), url_a),
        ("mod-b".to_string(), url_b),
    ]);

    let http_client = reqwest::Client::new();
    let outcome = activate_tenant(
        &pool,
        tenant_id,
        &module_codes,
        &registry,
        &http_client,
        Duration::from_secs(10),
    )
    .await
    .expect("activate_tenant should succeed");

    assert_eq!(
        outcome.checks["status"].as_str().unwrap(),
        "active",
        "all modules ready → status must be active; checks: {}",
        outcome.checks
    );

    let status = tenant_status(&pool, tenant_id).await;
    assert_eq!(
        status.as_deref(),
        Some("active"),
        "tenant row must be active in DB"
    );

    // provisioning_outbox must have a tenant.provisioned event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provisioning_outbox \
         WHERE tenant_id = $1 AND event_type = 'tenant.provisioned'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count outbox events");
    assert_eq!(outbox_count, 1, "must have exactly one tenant.provisioned event");

    cleanup(&pool, tenant_id).await;
    println!("✅ activate_tenant_polls_ready_all_up_becomes_active");
}

// ============================================================================
// Test 2: one module never returns up within timeout → tenant degraded
// ============================================================================

#[tokio::test]
async fn activate_tenant_polls_ready_timeout_causes_degraded() {
    let pool = registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = insert_provisioning_tenant(&pool).await;

    // mod-a: immediately ready; mod-b: always warming (never becomes up)
    let (url_a, _state_a) = start_module_server(true, tenant_id).await;
    let (url_b, _state_b) = start_module_server(false, tenant_id).await;

    let module_codes: Vec<String> = vec!["mod-a".to_string(), "mod-b".to_string()];
    let registry = build_test_registry(vec![
        ("mod-a".to_string(), url_a),
        ("mod-b".to_string(), url_b),
    ]);

    let http_client = reqwest::Client::new();
    // Short timeout so the test completes quickly
    let outcome = activate_tenant(
        &pool,
        tenant_id,
        &module_codes,
        &registry,
        &http_client,
        Duration::from_secs(2),
    )
    .await
    .expect("activate_tenant should return Ok even when degraded");

    assert_eq!(
        outcome.checks["status"].as_str().unwrap(),
        "degraded",
        "one module warming → status must be degraded; checks: {}",
        outcome.checks
    );

    let failed: Vec<Value> = outcome.checks["failed_modules"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        failed.iter().any(|m| m.as_str() == Some("mod-b")),
        "mod-b must be listed in failed_modules: {:?}",
        failed
    );
    assert!(
        !failed.iter().any(|m| m.as_str() == Some("mod-a")),
        "mod-a must NOT be in failed_modules: {:?}",
        failed
    );

    let status = tenant_status(&pool, tenant_id).await;
    assert_eq!(
        status.as_deref(),
        Some("degraded"),
        "tenant row must be degraded in DB"
    );

    // cp_tenant_module_status: mod-b must be failed
    let statuses = module_statuses(&pool, tenant_id).await;
    assert_eq!(
        statuses.get("mod-b").map(|s| s.as_str()),
        Some("failed"),
        "mod-b must be failed in cp_tenant_module_status; all: {:?}",
        statuses
    );

    cleanup(&pool, tenant_id).await;
    println!("✅ activate_tenant_polls_ready_timeout_causes_degraded");
}

// ============================================================================
// Test 3: poll_module_readiness for only the failing modules
//         (core of the degraded-retry flow)
// ============================================================================

#[tokio::test]
async fn poll_module_readiness_re_polls_failing_modules_only() {
    let pool = registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = insert_provisioning_tenant(&pool).await;

    // Simulate a degraded tenant: mod-a ready, mod-b failed in DB
    sqlx::query(
        "INSERT INTO cp_tenant_module_status \
         (tenant_id, module_code, status, created_at, updated_at) \
         VALUES ($1, 'mod-a', 'ready', NOW(), NOW()), \
                ($1, 'mod-b', 'failed', NOW(), NOW()) \
         ON CONFLICT (tenant_id, module_code) DO UPDATE \
         SET status = EXCLUDED.status, updated_at = NOW()",
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("seed module statuses");

    sqlx::query("UPDATE tenants SET status = 'degraded' WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("set degraded");

    // mod-b server now returns "up" (module recovered)
    let (url_b, _state_b) = start_module_server(true, tenant_id).await;

    let failed_modules = vec!["mod-b".to_string()];
    let registry = build_test_registry(vec![("mod-b".to_string(), url_b)]);

    let http_client = reqwest::Client::new();
    let still_failing = poll_module_readiness(
        tenant_id,
        &failed_modules,
        &registry,
        &http_client,
        Duration::from_secs(5),
    )
    .await;

    assert!(
        still_failing.is_empty(),
        "mod-b recovered — still_failing must be empty, got: {:?}",
        still_failing
    );

    // Simulate what the retry handler does: mark mod-b ready, promote to active
    let now = chrono::Utc::now();
    sqlx::query(
        "UPDATE cp_tenant_module_status SET status = 'ready', updated_at = $1 \
         WHERE tenant_id = $2 AND module_code = 'mod-b'",
    )
    .bind(now)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("mark mod-b ready");

    sqlx::query("UPDATE tenants SET status = 'active', updated_at = $1 WHERE tenant_id = $2")
        .bind(now)
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("promote to active");

    let status = tenant_status(&pool, tenant_id).await;
    assert_eq!(
        status.as_deref(),
        Some("active"),
        "after re-poll + status update, tenant must be active"
    );

    let statuses = module_statuses(&pool, tenant_id).await;
    assert_eq!(
        statuses.get("mod-b").map(|s| s.as_str()),
        Some("ready"),
        "mod-b must be ready after re-poll"
    );

    cleanup(&pool, tenant_id).await;
    println!("✅ poll_module_readiness_re_polls_failing_modules_only");
}
