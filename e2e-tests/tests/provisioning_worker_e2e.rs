//! E2E tests for the provisioning bundle worker and NATS outbox path.
//!
//! Verifies:
//! 1. A provisioning event written to `provisioning_outbox` can be published
//!    to NATS and received by a subscriber (outbox → NATS roundtrip).
//! 2. `provision_all_modules` provisions 3 modules (ar, gl, ap) and all
//!    reach `status = ready` in `cp_tenant_module_status`.
//!
//! No mocks. No stubs. Real NATS, real Postgres.
//!
//! Required services:
//!   - Tenant registry:  TENANT_REGISTRY_DATABASE_URL (default localhost:5441)
//!   - AR module:        AR_POSTGRES_HOST/PORT/USER/PASSWORD (default localhost:5434)
//!   - GL module:        GL_POSTGRES_HOST/PORT/USER/PASSWORD (default localhost:5438)
//!   - AP module:        AP_POSTGRES_HOST/PORT/USER/PASSWORD (default localhost:5443)
//!   - NATS:             NATS_URL (default nats://platform:dev-nats-token@localhost:4222)

mod common;

use common::get_tenant_registry_pool;
use control_plane::provisioning::{
    registry::{default_http_port_for, ModuleProvisioningConfig, ModuleRegistry},
    worker,
};
use futures::StreamExt;
use sqlx::{Connection, PgPool};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Absolute path to the workspace root (one level above e2e-tests/).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // e2e-tests/ → workspace root
        .expect("workspace root")
        .to_path_buf()
}

fn default_port_for(module_code: &str) -> u16 {
    match module_code {
        "ar" => 5434,
        "gl" => 5438,
        "ap" => 5443,
        _ => 5432,
    }
}

/// Build a `ModuleRegistry` pointing at the real local postgres instances.
fn build_registry(module_codes: &[&str]) -> ModuleRegistry {
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

/// Insert a test tenant and a bundle with the given modules.
/// Returns (tenant_id, bundle_id).
async fn seed_tenant_with_bundle(pool: &PgPool, module_codes: &[&str]) -> (Uuid, Uuid) {
    let tenant_id = Uuid::new_v4();
    let app_id = format!("app-e2e-{}", &tenant_id.to_string()[..8]);

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
         VALUES ('starter', 'e2e-prov-bundle', false) \
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

/// Drop per-tenant databases created by the worker during the test.
async fn drop_tenant_databases(tenant_id: Uuid, module_codes: &[&str]) {
    for code in module_codes {
        let db_name = control_plane::provisioning::steps::tenant_db_name(tenant_id, code);
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

/// Remove all test rows from the tenant-registry tables.
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

// ============================================================================
// Test 1: Provisioning outbox → NATS roundtrip
// ============================================================================

/// A provisioning event written to `provisioning_outbox` can be published to
/// NATS and received by a subscriber.
///
/// ## What this tests
/// 1. The DB outbox write succeeds (provisioning API guard → mutation → outbox).
/// 2. The NATS subject (`tenant.provisioning_started`) is reachable.
/// 3. The event payload round-trips correctly through the broker.
///
/// ## What it does NOT test
/// The outbox relay process (which reads `provisioning_outbox.published_at = null`
/// rows and publishes them). In tests the relay is not running; we publish
/// directly to NATS to simulate its action. If you need relay E2E coverage,
/// ensure the relay process is running alongside the test suite.
#[tokio::test]
async fn provisioning_outbox_event_arrives_on_nats() {
    let pool = get_tenant_registry_pool().await;
    let tenant_id = Uuid::new_v4();

    // Step 1: Write a provisioning outbox event (mirrors what the
    // provisioning API handler does on POST /api/control/tenants).
    sqlx::query(
        "INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at) \
         VALUES ($1, 'tenant.provisioning_started', $2, NOW())",
    )
    .bind(tenant_id)
    .bind(serde_json::json!({"tenant_id": tenant_id.to_string()}))
    .execute(&pool)
    .await
    .expect("write provisioning outbox event");

    // Confirm the event is in the DB (published_at = null = not yet relayed).
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provisioning_outbox \
         WHERE tenant_id = $1 AND event_type = 'tenant.provisioning_started'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count outbox events");
    assert_eq!(count, 1, "exactly one outbox event must exist in the DB");

    // Step 2: Connect to NATS and subscribe before publishing so no messages
    // are missed.
    let nats = common::setup_nats_client().await;
    let mut sub = nats
        .subscribe("tenant.provisioning_started")
        .await
        .expect("subscribe to tenant.provisioning_started");

    // Step 3: Publish the event payload directly to NATS, simulating what
    // the provisioning outbox relay does after it reads the DB row.
    let payload_bytes = serde_json::to_vec(&serde_json::json!({
        "tenant_id": tenant_id.to_string(),
    }))
    .unwrap();
    nats.publish(
        "tenant.provisioning_started".to_string(),
        payload_bytes.into(),
    )
    .await
    .expect("publish provisioning event to NATS");
    nats.flush().await.expect("flush NATS connection");

    // Step 4: Verify the message arrives within 3 seconds.
    let received = timeout(Duration::from_secs(3), sub.next()).await;

    match received {
        Ok(Some(msg)) => {
            let evt: serde_json::Value = serde_json::from_slice(&msg.payload)
                .expect("NATS message payload must be valid JSON");
            assert_eq!(
                evt["tenant_id"].as_str().unwrap(),
                tenant_id.to_string(),
                "NATS message must carry the provisioning tenant_id"
            );
            println!("✅ provisioning_outbox_event_arrives_on_nats: tenant_id={tenant_id}");
        }
        Ok(None) => panic!("NATS subscriber closed before receiving the provisioning event"),
        Err(_) => panic!(
            "timed out waiting for tenant.provisioning_started on NATS after 3 seconds — \
             check NATS connectivity (NATS_URL={:?})",
            std::env::var("NATS_URL").ok()
        ),
    }

    // Cleanup
    sqlx::query("DELETE FROM provisioning_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Test 2: Worker provisions 3 modules — all reach ready
// ============================================================================

/// Provision a bundle with 3 modules (ar, gl, ap). All must reach
/// `status = ready` in `cp_tenant_module_status`.
///
/// This is the top-level E2E counterpart to the control-plane unit test
/// in `platform/control-plane/tests/provisioning_worker.rs`, executed as
/// part of the main e2e suite so CI always runs it against live services.
#[tokio::test]
async fn worker_provisions_three_modules_all_reach_ready() {
    let pool = get_tenant_registry_pool().await;
    let module_codes = ["ar", "gl", "ap"];
    let (tenant_id, bundle_id) = seed_tenant_with_bundle(&pool, &module_codes).await;

    let module_codes_owned: Vec<String> = module_codes.iter().map(|s| s.to_string()).collect();
    let registry = build_registry(&module_codes);

    let result = worker::provision_all_modules(&pool, &registry, tenant_id, &module_codes_owned)
        .await
        .expect("provision_all_modules");

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
    assert!(result.all_ready(), "all_ready() must return true");

    // Verify per-module status rows directly in the database.
    let statuses = worker::module_statuses(&pool, tenant_id)
        .await
        .expect("read module statuses");

    assert_eq!(statuses.len(), 3, "must have a status row for each module");
    for (code, status) in &statuses {
        assert_eq!(status, "ready", "module {code} must be ready, got {status}");
    }

    drop_tenant_databases(tenant_id, &module_codes).await;
    cleanup(&pool, tenant_id, bundle_id).await;

    println!("✅ worker_provisions_three_modules_all_reach_ready: ar, gl, ap all status=ready");
}
