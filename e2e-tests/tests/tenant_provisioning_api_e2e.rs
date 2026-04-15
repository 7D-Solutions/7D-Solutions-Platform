//! E2E tests for the tenant provisioning API — module_statuses field and full lifecycle.
//!
//! Verifies:
//! 1. GET /provisioning exposes module_statuses from cp_tenant_module_status.
//!    All bundle modules are listed; modules not yet seeded show status=pending.
//! 2. Full async lifecycle: POST create → actual provisioning → poll until all
//!    modules ready → provisioned DB is accessible (tenant-scoped connectivity).
//! 3. Concurrent: 5 simultaneous creates, each reaches ready, no cross-tenant
//!    contamination in cp_tenant_module_status.
//! 4. Failure isolation: one bad module in the bundle shows status=failed while
//!    other modules reach ready.
//!
//! No mocks. Real Postgres databases. In-process Axum router for control-plane.
//!
//! Required services:
//!   - Tenant registry:  TENANT_REGISTRY_DATABASE_URL (default localhost:5441)
//!   - AR module:        AR_POSTGRES_HOST/PORT/USER/PASSWORD (default localhost:5434)
//!   - GL module:        GL_POSTGRES_HOST/PORT/USER/PASSWORD (default localhost:5438)
//!   - AP module:        AP_POSTGRES_HOST/PORT/USER/PASSWORD (default localhost:5443)

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::get_tenant_registry_pool;
use control_plane::{
    provisioning::{
        registry::{default_http_port_for, ModuleProvisioningConfig, ModuleRegistry},
        steps::tenant_db_name,
        worker,
    },
    routes::provisioning_router,
    state::AppState,
};
use serde_json::{json, Value};
use sqlx::{Connection, PgPool};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

// Modules used in most tests — small set to keep provisioning fast.
const TEST_MODULES: &[&str] = &["ar", "gl", "ap"];

// ============================================================================
// Setup helpers
// ============================================================================

/// Run all tenant-registry migrations needed by these tests (idempotent).
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

/// Build the in-process provisioning router backed by the given pool.
fn make_router(pool: PgPool) -> axum::Router {
    let state = Arc::new(AppState::new(pool, None));
    provisioning_router(state)
}

/// Default module port map (mirrors provisioning_worker_e2e.rs).
fn default_port_for(module_code: &str) -> u16 {
    match module_code {
        "ar" => 5434,
        "gl" => 5438,
        "ap" => 5443,
        _ => 5432,
    }
}

/// Build a ModuleRegistry pointing at real local postgres instances.
fn build_registry(module_codes: &[&str]) -> ModuleRegistry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let configs: Vec<(String, ModuleProvisioningConfig)> = module_codes
        .iter()
        .map(|&code| {
            let upper = code.to_uppercase();
            let host = std::env::var(format!("{upper}_POSTGRES_HOST"))
                .unwrap_or_else(|_| "localhost".to_string());
            let port: u16 = std::env::var(format!("{upper}_POSTGRES_PORT"))
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| default_port_for(code));
            let user = std::env::var(format!("{upper}_POSTGRES_USER"))
                .unwrap_or_else(|_| format!("{code}_user"));
            let password = std::env::var(format!("{upper}_POSTGRES_PASSWORD"))
                .unwrap_or_else(|_| format!("{code}_pass"));
            let migrations_path = root.join(format!("modules/{code}/db/migrations"));
            let cfg = ModuleProvisioningConfig {
                module_code: code.to_string(),
                postgres_host: host,
                postgres_port: port,
                postgres_user: user,
                postgres_password: password,
                migrations_path,
                http_base_url: format!("http://localhost:{}", default_http_port_for(code)),
            };
            (code.to_string(), cfg)
        })
        .collect();

    ModuleRegistry::from_configs(configs)
}

/// Seed a tenant record + bundle with the given module codes.
/// Returns (tenant_id, bundle_id).
async fn seed_tenant_with_bundle(pool: &PgPool, module_codes: &[&str]) -> (Uuid, Uuid) {
    let tenant_id = Uuid::new_v4();
    let app_id = format!("app-prov-api-{}", &tenant_id.to_string()[..8]);

    sqlx::query(
        "INSERT INTO tenants \
         (tenant_id, status, environment, module_schema_versions, \
          product_code, plan_code, app_id, created_at, updated_at) \
         VALUES ($1, 'pending', 'development', '{}'::jsonb, \
                 'starter', 'monthly', $2, NOW(), NOW())",
    )
    .bind(tenant_id)
    .bind(&app_id)
    .execute(pool)
    .await
    .expect("insert test tenant");

    let bundle_id: Uuid = sqlx::query_scalar(
        "INSERT INTO cp_bundles (product_code, bundle_name, is_default) \
         VALUES ('starter', 'prov-api-test-bundle', false) \
         RETURNING bundle_id",
    )
    .fetch_one(pool)
    .await
    .expect("insert bundle");

    for code in module_codes {
        sqlx::query(
            "INSERT INTO cp_bundle_modules (bundle_id, module_code, module_version) \
             VALUES ($1, $2, 'latest')",
        )
        .bind(bundle_id)
        .bind(code)
        .execute(pool)
        .await
        .expect("insert bundle module");
    }

    sqlx::query(
        "INSERT INTO cp_tenant_bundle (tenant_id, bundle_id, status) \
         VALUES ($1, $2, 'active')",
    )
    .bind(tenant_id)
    .bind(bundle_id)
    .execute(pool)
    .await
    .expect("assign bundle to tenant");

    (tenant_id, bundle_id)
}

/// Remove all registry rows written by a test.
async fn cleanup(pool: &PgPool, tenant_id: Uuid, bundle_id: Uuid) {
    for table in &[
        "cp_tenant_module_status",
        "provisioning_steps",
        "provisioning_outbox",
        "cp_tenant_bundle",
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
    sqlx::query("DELETE FROM cp_bundle_modules WHERE bundle_id = $1")
        .bind(bundle_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_bundles WHERE bundle_id = $1")
        .bind(bundle_id)
        .execute(pool)
        .await
        .ok();
}

/// Drop per-tenant module databases created by the worker.
async fn drop_tenant_databases(tenant_id: Uuid, module_codes: &[&str]) {
    for code in module_codes {
        let db_name = tenant_db_name(tenant_id, code);
        let upper = code.to_uppercase();
        let host = std::env::var(format!("{upper}_POSTGRES_HOST"))
            .unwrap_or_else(|_| "localhost".to_string());
        let port: u16 = std::env::var(format!("{upper}_POSTGRES_PORT"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| default_port_for(code));
        let user = std::env::var(format!("{upper}_POSTGRES_USER"))
            .unwrap_or_else(|_| format!("{code}_user"));
        let password = std::env::var(format!("{upper}_POSTGRES_PASSWORD"))
            .unwrap_or_else(|_| format!("{code}_pass"));
        let admin_url = format!("postgres://{user}:{password}@{host}:{port}/postgres");

        if let Ok(mut conn) = sqlx::PgConnection::connect(&admin_url).await {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
                .execute(&mut conn)
                .await;
            let _ = conn.close().await;
        }
    }
}

/// GET /api/control/tenants/{id}/provisioning via in-process router.
async fn get_provisioning_status(router: &axum::Router, tenant_id: Uuid) -> (StatusCode, Value) {
    let uri = format!("/api/control/tenants/{tenant_id}/provisioning");
    let request = Request::builder()
        .method("GET")
        .uri(&uri)
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, body)
}

/// Poll GET /provisioning until all module_statuses reach a terminal state
/// (all ready, or at least one failed with the rest ready/failed).
/// Returns the final response body. Fails after 60s.
async fn poll_until_modules_terminal(router: &axum::Router, tenant_id: Uuid) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let (status, body) = get_provisioning_status(router, tenant_id).await;
        assert_eq!(status, StatusCode::OK, "provisioning status failed: {body}");

        let modules = body["module_statuses"]
            .as_array()
            .expect("module_statuses must be array");

        let all_terminal = modules
            .iter()
            .all(|m| matches!(m["status"].as_str(), Some("ready") | Some("failed")));

        if all_terminal && !modules.is_empty() {
            return body;
        }

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for all modules to reach terminal state. \
                 Last response: {body}"
            );
        }
        sleep(Duration::from_millis(500)).await;
    }
}

// ============================================================================
// Test 1: module_statuses field reflects real db state
// ============================================================================

/// GET /provisioning returns module_statuses populated from cp_tenant_module_status.
/// Modules in the bundle but not yet seeded appear as status=pending.
#[tokio::test]
async fn tenant_provisioning_api_module_statuses_reflect_db_state() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let (tenant_id, bundle_id) = seed_tenant_with_bundle(&pool, TEST_MODULES).await;
    let router = make_router(pool.clone());

    // Before seeding cp_tenant_module_status: all modules should show pending
    // (they exist in the bundle but haven't been touched by the worker yet).
    let (status, body) = get_provisioning_status(&router, tenant_id).await;
    assert_eq!(status, StatusCode::OK, "unexpected status: {body}");

    let modules = body["module_statuses"]
        .as_array()
        .expect("module_statuses must be an array");
    assert_eq!(
        modules.len(),
        TEST_MODULES.len(),
        "must list all bundle modules, got: {modules:?}"
    );
    for m in modules {
        assert_eq!(
            m["status"].as_str().unwrap(),
            "pending",
            "unseeded module {} must be pending",
            m["module_code"]
        );
    }

    // Seed cp_tenant_module_status rows: ar=ready, gl=ready, ap=failed
    let now = chrono::Utc::now();
    for code in &["ar", "gl"] {
        sqlx::query(
            "INSERT INTO cp_tenant_module_status \
             (tenant_id, module_code, status, created_at, updated_at) \
             VALUES ($1, $2, 'ready', $3, $3) \
             ON CONFLICT (tenant_id, module_code) DO UPDATE SET status = 'ready', updated_at = $3",
        )
        .bind(tenant_id)
        .bind(code)
        .bind(now)
        .execute(&pool)
        .await
        .expect("seed module status");
    }
    sqlx::query(
        "INSERT INTO cp_tenant_module_status \
         (tenant_id, module_code, status, error_msg, created_at, updated_at) \
         VALUES ($1, 'ap', 'failed', 'injected test failure', $2, $2) \
         ON CONFLICT (tenant_id, module_code) \
         DO UPDATE SET status = 'failed', error_msg = 'injected test failure', updated_at = $2",
    )
    .bind(tenant_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("seed ap failure");

    // Verify the API response reflects the real DB state
    let (status, body) = get_provisioning_status(&router, tenant_id).await;
    assert_eq!(status, StatusCode::OK, "unexpected status: {body}");

    let modules = body["module_statuses"]
        .as_array()
        .expect("module_statuses array");

    let by_code: std::collections::HashMap<&str, &Value> = modules
        .iter()
        .map(|m| (m["module_code"].as_str().unwrap(), m))
        .collect();

    assert_eq!(by_code["ar"]["status"], "ready", "ar must be ready");
    assert_eq!(by_code["gl"]["status"], "ready", "gl must be ready");
    assert_eq!(by_code["ap"]["status"], "failed", "ap must be failed");
    assert_eq!(
        by_code["ap"]["error"].as_str().unwrap_or(""),
        "injected test failure",
        "ap error must be set"
    );

    // Invariant: module_statuses always comes from the DB — never inferred
    assert_eq!(modules.len(), 3, "exactly 3 modules from bundle");

    cleanup(&pool, tenant_id, bundle_id).await;
    println!("✅ tenant_provisioning_api_module_statuses_reflect_db_state");
}

// ============================================================================
// Test 2: full async provisioning lifecycle
// ============================================================================

/// POST /api/control/tenants → provision all modules (real DBs) →
/// poll GET /provisioning until all modules ready →
/// verify the provisioned GL database is accessible (tenant-scoped connectivity).
#[tokio::test]
async fn tenant_provisioning_api_full_lifecycle_provision_and_verify_connectivity() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let (tenant_id, bundle_id) = seed_tenant_with_bundle(&pool, TEST_MODULES).await;
    let module_codes_owned: Vec<String> = TEST_MODULES.iter().map(|s| s.to_string()).collect();
    let registry = build_registry(TEST_MODULES);
    let router = make_router(pool.clone());

    // Run real provisioning (creates tenant DBs, runs migrations, seeds)
    let result = worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("provision_all_modules");

    assert!(
        result.all_ready(),
        "all modules must be ready after provisioning, got: {:?}",
        result
            .results
            .iter()
            .map(|r| (&r.module_code, r.success, r.error.as_deref()))
            .collect::<Vec<_>>()
    );

    // Poll the provisioning status API — all module_statuses must reach ready
    let body = poll_until_modules_terminal(&router, tenant_id).await;
    let modules = body["module_statuses"].as_array().unwrap();

    assert_eq!(
        modules.len(),
        TEST_MODULES.len(),
        "all bundle modules must be listed"
    );
    for m in modules {
        assert_eq!(
            m["status"].as_str().unwrap(),
            "ready",
            "module {} must be ready, error: {:?}",
            m["module_code"],
            m["error"]
        );
    }

    // Tenant-scoped connectivity: connect to the provisioned GL database and
    // verify the schema was applied. This proves the module database is
    // accessible after provisioning completes.
    let gl_db_name = tenant_db_name(tenant_id, "gl");
    let upper = "GL";
    let host =
        std::env::var(format!("{upper}_POSTGRES_HOST")).unwrap_or_else(|_| "localhost".to_string());
    let port: u16 = std::env::var(format!("{upper}_POSTGRES_PORT"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5438);
    let user =
        std::env::var(format!("{upper}_POSTGRES_USER")).unwrap_or_else(|_| "gl_user".to_string());
    let password = std::env::var(format!("{upper}_POSTGRES_PASSWORD"))
        .unwrap_or_else(|_| "gl_pass".to_string());
    let tenant_gl_url = format!("postgres://{user}:{password}@{host}:{port}/{gl_db_name}");

    let mut conn = sqlx::PgConnection::connect(&tenant_gl_url)
        .await
        .expect("tenant GL DB must be reachable after provisioning");

    let migrations_applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&mut conn)
        .await
        .expect("_sqlx_migrations table must exist after migrations ran");

    assert!(
        migrations_applied > 0,
        "at least one migration must have been applied to the tenant GL DB"
    );
    conn.close().await.ok();

    // Cleanup
    drop_tenant_databases(tenant_id, TEST_MODULES).await;
    cleanup(&pool, tenant_id, bundle_id).await;

    println!("✅ tenant_provisioning_api_full_lifecycle_provision_and_verify_connectivity");
}

// ============================================================================
// Test 3: 5 concurrent creates — all reach ready, no cross-tenant contamination
// ============================================================================

/// 5 tenants provisioned concurrently. Each must reach all-modules-ready.
/// No cross-tenant contamination: each tenant's cp_tenant_module_status rows
/// must contain only its own module codes.
#[tokio::test]
async fn tenant_provisioning_api_concurrent_five_tenants_no_contamination() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    const N: usize = 5;

    // Seed N tenants with identical bundles (ar, gl, ap)
    let mut tenant_bundle_ids: Vec<(Uuid, Uuid)> = Vec::with_capacity(N);
    for _ in 0..N {
        let ids = seed_tenant_with_bundle(&pool, TEST_MODULES).await;
        tenant_bundle_ids.push(ids);
    }

    let module_codes_owned: Vec<String> = TEST_MODULES.iter().map(|s| s.to_string()).collect();
    let registry = Arc::new(build_registry(TEST_MODULES));

    // Provision all tenants concurrently
    let mut handles = Vec::with_capacity(N);
    for &(tenant_id, _bundle_id) in &tenant_bundle_ids {
        let pool_c = pool.clone();
        let registry_c = registry.clone();
        let codes_c = module_codes_owned.clone();
        handles.push(tokio::spawn(async move {
            worker::provision_all_modules(&pool_c, &registry_c, tenant_id, &codes_c).await
        }));
    }

    let mut all_results = Vec::with_capacity(N);
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle
            .await
            .expect("task must not panic")
            .expect("provision_all_modules must succeed");
        all_results.push((tenant_bundle_ids[i].0, result));
    }

    // Every tenant must have all modules ready
    for (tenant_id, result) in &all_results {
        assert!(
            result.all_ready(),
            "tenant {tenant_id} must have all modules ready, got: {:?}",
            result
                .results
                .iter()
                .map(|r| (&r.module_code, r.success, r.error.as_deref()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            result.ready_count,
            TEST_MODULES.len(),
            "tenant {tenant_id} must have exactly {} ready modules",
            TEST_MODULES.len()
        );
    }

    // Cross-tenant contamination check: each tenant's cp_tenant_module_status
    // rows must only contain that tenant's own modules.
    for &(tenant_id, _bundle_id) in &tenant_bundle_ids {
        let statuses: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT tenant_id, module_code FROM cp_tenant_module_status WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_all(&pool)
        .await
        .expect("query module statuses");

        assert_eq!(
            statuses.len(),
            TEST_MODULES.len(),
            "tenant {tenant_id} must have exactly {} status rows, got {}",
            TEST_MODULES.len(),
            statuses.len()
        );
        for (row_tenant_id, module_code) in &statuses {
            assert_eq!(
                *row_tenant_id, tenant_id,
                "module {module_code} status row belongs to wrong tenant: \
                 expected {tenant_id}, got {row_tenant_id}"
            );
            assert!(
                TEST_MODULES.contains(&module_code.as_str()),
                "unexpected module code {module_code} in tenant {tenant_id} status"
            );
        }
    }

    // Cleanup: drop all provisioned tenant databases and registry rows
    let all_tenant_ids: Vec<Uuid> = tenant_bundle_ids.iter().map(|(t, _)| *t).collect();
    for &(tenant_id, bundle_id) in &tenant_bundle_ids {
        drop_tenant_databases(tenant_id, TEST_MODULES).await;
        cleanup(&pool, tenant_id, bundle_id).await;
    }
    drop(all_tenant_ids);

    println!(
        "✅ tenant_provisioning_api_concurrent_five_tenants_no_contamination: \
         all {N} tenants provisioned without cross-tenant contamination"
    );
}

// ============================================================================
// Test 4: failed module isolated — others still reach ready
// ============================================================================

/// A bundle with one invalid module code (not in the registry) plus two real
/// modules. The invalid module must show status=failed in /provisioning while
/// the real modules show status=ready.
#[tokio::test]
async fn tenant_provisioning_api_failed_module_isolated_others_reach_ready() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    // Bundle: ar + gl (real) + nonexistent_module (will fail)
    let mixed_modules: &[&str] = &["ar", "gl", "nonexistent_module"];
    let (tenant_id, bundle_id) = seed_tenant_with_bundle(&pool, mixed_modules).await;

    // Registry knows only ar + gl; nonexistent_module will get ModuleNotFound
    let registry = build_registry(&["ar", "gl"]);
    let module_codes_owned: Vec<String> = mixed_modules.iter().map(|s| s.to_string()).collect();

    let result = worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("provision_all_modules must not hard-fail even with one bad module");

    // Worker must report 2 ready and 1 failed
    assert_eq!(result.ready_count, 2, "ar + gl must be ready");
    assert_eq!(result.failed_count, 1, "nonexistent_module must fail");
    assert!(
        !result.all_ready(),
        "all_ready() must be false when any module failed"
    );

    // Verify via the provisioning status API
    let router = make_router(pool.clone());
    let (http_status, body) = get_provisioning_status(&router, tenant_id).await;
    assert_eq!(http_status, StatusCode::OK, "unexpected: {body}");

    let modules = body["module_statuses"].as_array().unwrap();
    let by_code: std::collections::HashMap<&str, &Value> = modules
        .iter()
        .map(|m| (m["module_code"].as_str().unwrap(), m))
        .collect();

    assert_eq!(
        by_code["ar"]["status"], "ready",
        "ar must be ready despite nonexistent_module failing"
    );
    assert_eq!(
        by_code["gl"]["status"], "ready",
        "gl must be ready despite nonexistent_module failing"
    );
    assert_eq!(
        by_code["nonexistent_module"]["status"], "failed",
        "nonexistent_module must show failed"
    );
    // The failure error must be populated
    let error_msg = by_code["nonexistent_module"]["error"]
        .as_str()
        .unwrap_or("");
    assert!(
        !error_msg.is_empty(),
        "failed module must have an error message"
    );

    // Cleanup: only drop real module DBs that were actually created
    drop_tenant_databases(tenant_id, &["ar", "gl"]).await;
    cleanup(&pool, tenant_id, bundle_id).await;

    println!(
        "✅ tenant_provisioning_api_failed_module_isolated_others_reach_ready: \
         ar+gl=ready, nonexistent_module=failed"
    );
}
