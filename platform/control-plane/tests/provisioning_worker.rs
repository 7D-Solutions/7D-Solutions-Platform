//! Integration tests for the provisioning bundle worker.
//!
//! Verifies that `provision_all_modules` provisions each module independently,
//! tracks per-module status in `cp_tenant_module_status`, and that all modules
//! reach `ready` status on a successful run.
//!
//! Requires real databases:
//!   - Tenant registry: TENANT_REGISTRY_DATABASE_URL (default localhost:5441)
//!   - AR module:       AR_POSTGRES_HOST/PORT (default localhost:5434)
//!   - GL module:       GL_POSTGRES_HOST/PORT (default localhost:5438)
//!   - AP module:       AP_POSTGRES_HOST/PORT (default localhost:5443)
//!
//! No mocks. No stubs.

use sqlx::{Connection, PgPool};
use std::path::PathBuf;
use uuid::Uuid;

use control_plane::provisioning::registry::{ModuleProvisioningConfig, ModuleRegistry};
use control_plane::provisioning::worker;

// ============================================================================
// Helpers
// ============================================================================

async fn registry_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    PgPool::connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

/// Absolute path to the workspace root (two levels above control-plane's Cargo.toml).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // platform/control-plane → platform
        .and_then(|p| p.parent()) // platform → workspace root
        .expect("workspace root")
        .to_path_buf()
}

/// Build a ModuleRegistry pointing at the real local module postgres instances.
fn test_registry(module_codes: &[&str]) -> ModuleRegistry {
    let root = workspace_root();

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

            let http_port = control_plane::provisioning::registry::default_http_port_for(code);
            let http_base_url = format!("http://localhost:{http_port}");
            let cfg = ModuleProvisioningConfig {
                module_code: code.to_string(),
                postgres_host: host,
                postgres_port: port,
                postgres_user: user,
                postgres_password: password,
                migrations_path,
                http_base_url,
            };
            (code.to_string(), cfg)
        })
        .collect();

    ModuleRegistry::from_configs(configs)
}

fn default_port_for(module_code: &str) -> u16 {
    match module_code {
        "ar" => 5434,
        "gl" => 5438,
        "ap" => 5443,
        _ => 5432,
    }
}

/// Insert a tenant and a bundle with the given modules.
/// Returns (tenant_id, bundle_id).
async fn seed_tenant_with_bundle(pool: &PgPool, module_codes: &[&str]) -> (Uuid, Uuid) {
    let tenant_id = Uuid::new_v4();
    let app_id = format!("app-worker-{}", &tenant_id.to_string()[..8]);

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
    .expect("insert tenant");

    let bundle_id: Uuid = sqlx::query_scalar(
        "INSERT INTO cp_bundles (product_code, bundle_name, is_default) \
         VALUES ('starter', 'test-bundle', false) \
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

/// Drop per-tenant databases created during the test.
async fn drop_tenant_databases(_pool: &PgPool, tenant_id: Uuid, module_codes: &[&str]) {
    for code in module_codes {
        let db_name = control_plane::provisioning::steps::tenant_db_name(tenant_id, code);

        // Use the module's admin connection to drop the database
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
        }
    }
}

/// Clean up test data from the tenant-registry.
async fn cleanup(pool: &PgPool, tenant_id: Uuid, bundle_id: Uuid) {
    sqlx::query("DELETE FROM cp_tenant_module_status WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM provisioning_steps WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM provisioning_outbox WHERE tenant_id = $1")
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

// ============================================================================
// Tests
// ============================================================================

/// Happy path: provision a bundle with 3 modules.
/// All 3 must reach status=ready in cp_tenant_module_status.
#[tokio::test]
async fn provision_three_modules_all_reach_ready() {
    let pool = registry_pool().await;
    let module_codes = ["ar", "gl", "ap"];
    let (tenant_id, bundle_id) = seed_tenant_with_bundle(&pool, &module_codes).await;

    let module_codes_owned: Vec<String> = module_codes.iter().map(|s| s.to_string()).collect();
    let registry = test_registry(&module_codes);

    // Seed module statuses and run the worker
    worker::seed_module_statuses(&pool, tenant_id, &module_codes_owned)
        .await
        .expect("seed module statuses");

    let result = worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("provision_all_modules");

    // All 3 modules must succeed
    assert_eq!(
        result.ready_count,
        3,
        "expected 3 ready modules, got {}: {:?}",
        result.ready_count,
        result
            .results
            .iter()
            .map(|r| (&r.module_code, r.success, r.error.as_deref()))
            .collect::<Vec<_>>()
    );
    assert_eq!(result.failed_count, 0, "no modules should have failed");
    assert!(result.all_ready(), "all_ready() should return true");

    // Verify status rows in the database
    let statuses = worker::module_statuses(&pool, tenant_id)
        .await
        .expect("read module statuses");

    assert_eq!(statuses.len(), 3, "should have 3 status rows");
    for (code, status) in &statuses {
        assert_eq!(
            status, "ready",
            "module {code} should be ready, got {status}"
        );
    }

    // Cleanup: drop tenant databases and registry rows
    drop_tenant_databases(&pool, tenant_id, &module_codes).await;
    cleanup(&pool, tenant_id, bundle_id).await;
}

/// Idempotency: running provision_all_modules twice skips already-ready modules.
#[tokio::test]
async fn provision_is_idempotent_for_ready_modules() {
    let pool = registry_pool().await;
    let module_codes = ["ar"];
    let (tenant_id, bundle_id) = seed_tenant_with_bundle(&pool, &module_codes).await;

    let module_codes_owned: Vec<String> = module_codes.iter().map(|s| s.to_string()).collect();
    let registry = test_registry(&module_codes);

    // First run
    worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("first provisioning");

    // Second run — should skip AR since it's already ready
    let result = worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("second provisioning");

    assert_eq!(result.ready_count, 1);
    assert_eq!(result.failed_count, 0);

    drop_tenant_databases(&pool, tenant_id, &module_codes).await;
    cleanup(&pool, tenant_id, bundle_id).await;
}

/// Partial failure: a module with an unreachable host is marked failed,
/// while other modules continue and reach ready.
#[tokio::test]
async fn partial_failure_marks_failed_module_and_continues_others() {
    let pool = registry_pool().await;

    let tenant_id = Uuid::new_v4();
    let app_id = format!("app-pf-{}", &tenant_id.to_string()[..8]);

    sqlx::query(
        "INSERT INTO tenants \
         (tenant_id, status, environment, module_schema_versions, \
          product_code, plan_code, app_id, created_at, updated_at) \
         VALUES ($1, 'pending', 'development', '{}'::jsonb, \
                 'starter', 'monthly', $2, NOW(), NOW())",
    )
    .bind(tenant_id)
    .bind(&app_id)
    .execute(&pool)
    .await
    .expect("insert tenant");

    let bundle_id: Uuid = sqlx::query_scalar(
        "INSERT INTO cp_bundles (product_code, bundle_name, is_default) \
         VALUES ('starter', 'pf-test-bundle', false) \
         RETURNING bundle_id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert bundle");

    // "ghost" module points to an unreachable host
    for code in &["ar", "ghost"] {
        sqlx::query(
            "INSERT INTO cp_bundle_modules (bundle_id, module_code, module_version) \
             VALUES ($1, $2, 'latest')",
        )
        .bind(bundle_id)
        .bind(code)
        .execute(&pool)
        .await
        .expect("insert bundle module");
    }

    sqlx::query(
        "INSERT INTO cp_tenant_bundle (tenant_id, bundle_id, status) \
         VALUES ($1, $2, 'active')",
    )
    .bind(tenant_id)
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("assign bundle");

    // Build a registry where "ghost" points to an unreachable host
    let ar_cfg = ModuleProvisioningConfig {
        module_code: "ar".to_string(),
        postgres_host: "localhost".to_string(),
        postgres_port: 5434,
        postgres_user: "ar_user".to_string(),
        postgres_password: "ar_pass".to_string(),
        migrations_path: workspace_root().join("modules/ar/db/migrations"),
        http_base_url: "http://localhost:8086".to_string(),
    };
    let ghost_cfg = ModuleProvisioningConfig {
        module_code: "ghost".to_string(),
        postgres_host: "127.0.0.1".to_string(),
        postgres_port: 19999, // nothing listens here
        postgres_user: "ghost_user".to_string(),
        postgres_password: "ghost_pass".to_string(),
        migrations_path: PathBuf::from("./modules/ghost/db/migrations"),
        http_base_url: "http://127.0.0.1:19998".to_string(),
    };
    let registry = ModuleRegistry::from_configs(vec![
        ("ar".to_string(), ar_cfg),
        ("ghost".to_string(), ghost_cfg),
    ]);

    let module_codes_owned = vec!["ar".to_string(), "ghost".to_string()];

    let result = worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("provision_all_modules returns Ok even on partial failure");

    assert_eq!(result.ready_count, 1, "ar should be ready");
    assert_eq!(result.failed_count, 1, "ghost should have failed");

    // Verify status rows
    let statuses = worker::module_statuses(&pool, tenant_id)
        .await
        .expect("read statuses");

    let ar_status = statuses
        .iter()
        .find(|(c, _)| c == "ar")
        .map(|(_, s)| s.as_str())
        .unwrap_or("missing");
    let ghost_status = statuses
        .iter()
        .find(|(c, _)| c == "ghost")
        .map(|(_, s)| s.as_str())
        .unwrap_or("missing");

    assert_eq!(ar_status, "ready", "ar should be ready");
    assert_eq!(ghost_status, "failed", "ghost should be failed");

    // Cleanup
    drop_tenant_databases(&pool, tenant_id, &["ar"]).await;
    sqlx::query("DELETE FROM cp_tenant_module_status WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM provisioning_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
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
}
