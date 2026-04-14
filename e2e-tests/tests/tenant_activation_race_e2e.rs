//! E2E: tenant activation race — tenant must not become active until all
//! module /api/ready?tenant_id= probes return up.
//!
//! GAP-16 integration test.  Uses in-process HTTP servers as module stubs;
//! the tenant-registry Postgres is real (no mocks there either).
//!
//! Required services:
//!   - Tenant registry: TENANT_REGISTRY_DATABASE_URL (default localhost:5441)

mod common;

use axum::{extract::Query, routing::get, Json, Router};
use chrono::Utc;
use common::get_tenant_registry_pool;
use control_plane::provisioning::{
    registry::{ModuleProvisioningConfig, ModuleRegistry},
    steps::activate_tenant,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use std::{path::PathBuf, sync::{Arc, Mutex}, time::Duration};
use tokio::net::TcpListener;
use uuid::Uuid;

// ============================================================================
// Shared helpers
// ============================================================================

async fn ensure_tables(pool: &PgPool) {
    let migrations: &[&str] = &[
        include_str!(
            "../../platform/tenant-registry/db/migrations/20260217000001_add_control_plane_tables.sql"
        ),
        include_str!(
            "../../platform/tenant-registry/db/migrations/20260219000001_extend_tenants_product_plan_status_appid.sql"
        ),
        include_str!(
            "../../platform/tenant-registry/db/migrations/20260219000002_add_entitlements.sql"
        ),
        include_str!(
            "../../platform/tenant-registry/db/migrations/20260219000003_add_cp_plans.sql"
        ),
        include_str!(
            "../../platform/tenant-registry/db/migrations/20260220000002_add_bundle_tables.sql"
        ),
        include_str!(
            "../../platform/tenant-registry/db/migrations/20260410000001_add_tenant_module_status.sql"
        ),
    ];
    for sql in migrations {
        sqlx::raw_sql(sql).execute(pool).await.ok();
    }
}

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

async fn get_tenant_status(pool: &PgPool, tenant_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .expect("query tenant status")
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
// In-process module server
// ============================================================================

#[derive(Clone)]
struct ModuleServerState {
    ready: Arc<Mutex<bool>>,
    tenant_id: Uuid,
}

#[derive(Deserialize)]
struct ReadyQuery {
    tenant_id: Option<Uuid>,
}

async fn start_module_server(initially_ready: bool, tenant_id: Uuid) -> (String, ModuleServerState) {
    let state = ModuleServerState {
        ready: Arc::new(Mutex::new(initially_ready)),
        tenant_id,
    };
    let state_clone = state.clone();

    let app = Router::new().route(
        "/api/ready",
        get(move |Query(q): Query<ReadyQuery>| {
            let s = state_clone.clone();
            async move {
                let is_ready = *s.ready.lock().unwrap();
                let queried_tid = q.tenant_id.unwrap_or(Uuid::nil());
                let tenant_status = if queried_tid == s.tenant_id && is_ready {
                    "up"
                } else {
                    "warming"
                };
                Json(json!({
                    "status": "ready",
                    "tenant": { "id": queried_tid.to_string(), "status": tenant_status }
                }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });

    (base_url, state)
}

fn build_test_registry(modules: Vec<(String, String)>) -> ModuleRegistry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let configs = modules
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
// Test 1: all modules ready → active
// ============================================================================

/// Provision a tenant. When all module probes immediately return `up`, the
/// tenant must be marked `active` — never before.
#[tokio::test]
async fn tenant_activation_race_all_ready_becomes_active() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = insert_provisioning_tenant(&pool).await;

    let (url_a, _) = start_module_server(true, tenant_id).await;
    let (url_b, _) = start_module_server(true, tenant_id).await;

    let module_codes = vec!["mod-a".to_string(), "mod-b".to_string()];
    let registry = build_test_registry(vec![
        ("mod-a".to_string(), url_a),
        ("mod-b".to_string(), url_b),
    ]);

    // Tenant must still be 'provisioning' before activation runs
    assert_eq!(
        get_tenant_status(&pool, tenant_id).await.as_deref(),
        Some("provisioning"),
        "precondition: tenant must be provisioning"
    );

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
    .expect("activate_tenant must succeed when all modules are up");

    assert_eq!(
        outcome.checks["status"].as_str().unwrap(),
        "active",
        "all ready → outcome status must be active"
    );

    assert_eq!(
        get_tenant_status(&pool, tenant_id).await.as_deref(),
        Some("active"),
        "tenant DB row must be active after all modules confirm ready"
    );

    cleanup(&pool, tenant_id).await;
    println!("✅ tenant_activation_race_all_ready_becomes_active");
}

// ============================================================================
// Test 2: one module never returns up → degraded, not active
// ============================================================================

/// The invariant: a tenant NEVER becomes active when any module is still
/// warming. If the timeout expires first, the tenant becomes degraded — and the
/// slow module is recorded in failed_modules.
#[tokio::test]
async fn tenant_activation_race_slow_module_causes_degraded_not_active() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = insert_provisioning_tenant(&pool).await;

    // mod-a: ready immediately; mod-b: never becomes up
    let (url_a, _) = start_module_server(true, tenant_id).await;
    let (url_b, _) = start_module_server(false, tenant_id).await;

    let module_codes = vec!["mod-a".to_string(), "mod-b".to_string()];
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
        Duration::from_secs(2), // short timeout so test finishes quickly
    )
    .await
    .expect("activate_tenant returns Ok even in degraded case");

    // Outcome must be degraded, not active
    assert_eq!(
        outcome.checks["status"].as_str().unwrap(),
        "degraded",
        "slow module → outcome status must be degraded, not active"
    );

    let empty = vec![];
    let failed: Vec<&str> = outcome.checks["failed_modules"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    assert!(
        failed.contains(&"mod-b"),
        "mod-b must be in failed_modules: {:?}",
        failed
    );
    assert!(
        !failed.contains(&"mod-a"),
        "mod-a must NOT be in failed_modules: {:?}",
        failed
    );

    // Critical invariant: DB status must be 'degraded', never 'active'
    assert_eq!(
        get_tenant_status(&pool, tenant_id).await.as_deref(),
        Some("degraded"),
        "tenant must be degraded in DB — NEVER active when a module is still warming"
    );

    // mod-b must be recorded as failed in cp_tenant_module_status
    let mod_b_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM cp_tenant_module_status \
         WHERE tenant_id = $1 AND module_code = 'mod-b'",
    )
    .bind(tenant_id)
    .fetch_optional(&pool)
    .await
    .expect("query mod-b status");

    assert_eq!(
        mod_b_status.as_deref(),
        Some("failed"),
        "mod-b must be marked failed in cp_tenant_module_status"
    );

    cleanup(&pool, tenant_id).await;
    println!("✅ tenant_activation_race_slow_module_causes_degraded_not_active");
}

// ============================================================================
// Test 3: no user call accepted before activation completes (ordering check)
// ============================================================================

/// Concurrency ordering: even when activate_tenant is run, the tenant status
/// is NOT updated until after all probes complete. Reading status BEFORE the
/// call completes must still show 'provisioning'.
///
/// This is verified by running the activation in a background task and reading
/// status mid-flight (while mod-b is still warming). The read must return
/// 'provisioning', not 'active'.
#[tokio::test]
async fn tenant_not_active_until_all_probes_complete() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = insert_provisioning_tenant(&pool).await;

    // mod-a: ready. mod-b: starts warming, becomes up after 1.5s.
    let (url_a, _) = start_module_server(true, tenant_id).await;
    let (url_b, state_b) = start_module_server(false, tenant_id).await;

    let module_codes = vec!["mod-a".to_string(), "mod-b".to_string()];
    let registry = build_test_registry(vec![
        ("mod-a".to_string(), url_a),
        ("mod-b".to_string(), url_b),
    ]);

    let pool_read = pool.clone();
    let http_client = reqwest::Client::new();

    // Spawn activation in background
    let pool_bg = pool.clone();
    let module_codes_bg = module_codes.clone();
    let registry_bg = registry.clone();
    let http_bg = http_client.clone();
    let activation_handle = tokio::spawn(async move {
        activate_tenant(
            &pool_bg,
            tenant_id,
            &module_codes_bg,
            &registry_bg,
            &http_bg,
            Duration::from_secs(10),
        )
        .await
    });

    // While activation is polling (mod-b warming), status must still be 'provisioning'
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mid_flight_status = get_tenant_status(&pool_read, tenant_id).await;
    assert_eq!(
        mid_flight_status.as_deref(),
        Some("provisioning"),
        "status must be 'provisioning' while mod-b is still warming — never 'active' early"
    );

    // Now flip mod-b to ready
    *state_b.ready.lock().unwrap() = true;

    // Wait for activation to complete
    let outcome = activation_handle
        .await
        .expect("activation task must not panic")
        .expect("activate_tenant must succeed after mod-b flips up");

    assert_eq!(
        outcome.checks["status"].as_str().unwrap(),
        "active",
        "after mod-b flips up, outcome must be active"
    );

    assert_eq!(
        get_tenant_status(&pool, tenant_id).await.as_deref(),
        Some("active"),
        "final DB status must be active"
    );

    cleanup(&pool, tenant_id).await;
    println!("✅ tenant_not_active_until_all_probes_complete");
}
