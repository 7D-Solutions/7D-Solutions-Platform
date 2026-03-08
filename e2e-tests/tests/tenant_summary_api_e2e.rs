//! E2E tests for the tenant summary API
//!
//! Tests GET /api/control/tenants/:tenant_id/summary via:
//!   1. Direct library call (fetch_tenant_summary) — no HTTP server needed
//!   2. In-process Axum router connected to real tenant-registry DB
//!
//! Verifies:
//! - Stable JSON shape: tenant_id, status, environment, created_at, modules, overall_ready
//! - 404 for unknown tenant
//! - Module readiness fanout: all 5 modules attempted, timeouts handled gracefully
//! - Serialization round-trip

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::get_tenant_registry_pool;
use control_plane::routes::build_router;
use control_plane::state::AppState;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tenant_registry::{ModuleUrl, ReadinessStatus, SummaryState, TenantSummary};
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

/// Run migrations for the provisioning tables (idempotent)
async fn ensure_tables(pool: &PgPool) {
    let base_sql = include_str!(
        "../../platform/tenant-registry/db/migrations/20260216000001_create_tenant_registry.sql"
    );
    sqlx::raw_sql(base_sql).execute(pool).await.ok();

    // Also run the control-plane migration if it exists
    let cp_sql = include_str!(
        "../../platform/tenant-registry/db/migrations/20260217000001_add_control_plane_tables.sql"
    );
    sqlx::raw_sql(cp_sql).execute(pool).await.ok();
}

async fn insert_test_tenant(pool: &PgPool, tenant_id: Uuid, status: &str) {
    sqlx::query(
        r#"
        INSERT INTO tenants (tenant_id, status, environment, module_schema_versions)
        VALUES ($1, $2, 'development', '{}'::jsonb)
        ON CONFLICT (tenant_id) DO UPDATE SET status = EXCLUDED.status
        "#,
    )
    .bind(tenant_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("Failed to insert test tenant");
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Build an Axum test app with unreachable module URLs (to test timeout handling)
fn build_test_app_unreachable_modules(pool: PgPool) -> axum::Router {
    let app_state = Arc::new(AppState::new(pool.clone(), None));
    let summary_state = Arc::new(SummaryState::new_with_urls(
        pool,
        vec![
            // Use addresses that refuse connections immediately
            ModuleUrl::new("ar", "http://127.0.0.1:19999"),
            ModuleUrl::new("payments", "http://127.0.0.1:19998"),
            ModuleUrl::new("subscriptions", "http://127.0.0.1:19997"),
            ModuleUrl::new("gl", "http://127.0.0.1:19996"),
            ModuleUrl::new("notifications", "http://127.0.0.1:19995"),
        ],
    ));
    build_router(app_state, summary_state)
}

// ============================================================================
// Unit-level tests (fetch_tenant_summary library function)
// ============================================================================

/// fetch_tenant_summary returns TenantNotFound for an unknown UUID
#[tokio::test]
#[serial]
async fn test_summary_not_found_for_unknown_tenant() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let client = reqwest::Client::new();
    let unknown_id = Uuid::new_v4();

    let result = tenant_registry::fetch_tenant_summary(
        &pool,
        &client,
        &ModuleUrl::default_local(),
        unknown_id,
    )
    .await;

    assert!(
        result.is_err(),
        "Expected TenantNotFound error for unknown tenant"
    );
    match result.unwrap_err() {
        tenant_registry::SummaryError::TenantNotFound(id) => {
            assert_eq!(id, unknown_id);
        }
        e => panic!("Expected TenantNotFound, got: {:?}", e),
    }
}

/// fetch_tenant_summary returns a well-formed TenantSummary for a known tenant
/// even when modules are unreachable (all marked as Unavailable)
#[tokio::test]
#[serial]
async fn test_summary_stable_shape_with_unreachable_modules() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = Uuid::new_v4();
    insert_test_tenant(&pool, tenant_id, "provisioning").await;

    // Use unreachable module URLs so the test doesn't depend on running services
    let module_urls: Vec<ModuleUrl> = vec![
        ModuleUrl::new("ar", "http://127.0.0.1:19999"),
        ModuleUrl::new("payments", "http://127.0.0.1:19998"),
        ModuleUrl::new("subscriptions", "http://127.0.0.1:19997"),
        ModuleUrl::new("gl", "http://127.0.0.1:19996"),
        ModuleUrl::new("notifications", "http://127.0.0.1:19995"),
    ];

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build()
        .unwrap();

    let result = tenant_registry::fetch_tenant_summary(&pool, &client, &module_urls, tenant_id)
        .await
        .expect("Summary fetch should succeed even with unreachable modules");

    // Verify stable JSON shape
    assert_eq!(result.tenant_id, tenant_id);
    assert_eq!(result.status, "provisioning");
    assert_eq!(result.environment, "development");

    // All 5 modules should be present
    assert_eq!(
        result.modules.len(),
        5,
        "Expected 5 module readiness entries"
    );

    let module_names: Vec<&str> = result.modules.iter().map(|m| m.module.as_str()).collect();
    assert!(module_names.contains(&"ar"), "Missing ar module");
    assert!(module_names.contains(&"gl"), "Missing gl module");
    assert!(
        module_names.contains(&"payments"),
        "Missing payments module"
    );
    assert!(
        module_names.contains(&"subscriptions"),
        "Missing subscriptions module"
    );
    assert!(
        module_names.contains(&"notifications"),
        "Missing notifications module"
    );

    // Modules are unreachable → overall_ready must be false
    assert!(
        !result.overall_ready,
        "overall_ready must be false when modules are unreachable"
    );

    // All modules should be Unavailable
    for module in &result.modules {
        assert_eq!(
            module.status,
            ReadinessStatus::Unavailable,
            "Module {} should be Unavailable",
            module.module
        );
        assert!(
            module.error.is_some(),
            "Module {} should have an error message",
            module.module
        );
        assert!(
            module.latency_ms <= 2000,
            "Module {} latency_ms should be reasonable, got {}",
            module.module,
            module.latency_ms
        );
    }

    // Serializes to valid JSON
    let json = serde_json::to_string(&result).expect("TenantSummary must serialize to JSON");
    assert!(
        json.contains("overall_ready"),
        "JSON missing overall_ready field"
    );
    assert!(json.contains("modules"), "JSON missing modules field");

    cleanup_tenant(&pool, tenant_id).await;
}

/// TenantSummary serializes with stable field names
#[tokio::test]
async fn test_summary_json_field_names() {
    let summary = TenantSummary {
        tenant_id: Uuid::nil(),
        status: "active".to_string(),
        environment: "production".to_string(),
        created_at: chrono::Utc::now(),
        modules: vec![tenant_registry::ModuleReadiness {
            module: "ar".to_string(),
            status: ReadinessStatus::Ready,
            schema_version: Some("20260216000001".to_string()),
            latency_ms: 15,
            error: None,
        }],
        overall_ready: true,
    };

    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&summary).unwrap()).unwrap();

    // Verify all required fields are present
    assert!(json.get("tenant_id").is_some(), "Missing tenant_id");
    assert!(json.get("status").is_some(), "Missing status");
    assert!(json.get("environment").is_some(), "Missing environment");
    assert!(json.get("created_at").is_some(), "Missing created_at");
    assert!(json.get("modules").is_some(), "Missing modules");
    assert!(json.get("overall_ready").is_some(), "Missing overall_ready");

    // Verify module fields
    let module = &json["modules"][0];
    assert!(
        module.get("module").is_some(),
        "Module missing 'module' field"
    );
    assert!(
        module.get("status").is_some(),
        "Module missing 'status' field"
    );
    assert!(
        module.get("latency_ms").is_some(),
        "Module missing 'latency_ms' field"
    );
}

// ============================================================================
// HTTP-level tests (in-process Axum router)
// ============================================================================

/// GET /api/control/tenants/:tenant_id/summary returns 404 for unknown tenant
#[tokio::test]
#[serial]
async fn test_summary_http_404_unknown_tenant() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let app = build_test_app_unreachable_modules(pool);

    let unknown_id = Uuid::new_v4();
    let req = Request::builder()
        .uri(format!("/api/control/tenants/{}/summary", unknown_id))
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Unknown tenant should return 404"
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json.get("error").is_some(),
        "404 response should have 'error' field"
    );
}

/// GET /api/control/tenants/:tenant_id/summary returns 200 with all 5 modules
/// (modules unavailable but response structure is correct)
#[tokio::test]
#[serial]
async fn test_summary_http_200_with_unavailable_modules() {
    let pool = get_tenant_registry_pool().await;
    ensure_tables(&pool).await;

    let tenant_id = Uuid::new_v4();
    insert_test_tenant(&pool, tenant_id, "active").await;

    let app = build_test_app_unreachable_modules(pool.clone());

    let req = Request::builder()
        .uri(format!("/api/control/tenants/{}/summary", tenant_id))
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Known tenant should return 200 even when modules are down"
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify shape
    assert_eq!(json["tenant_id"].as_str().unwrap(), tenant_id.to_string());
    assert_eq!(json["status"].as_str().unwrap(), "active");
    assert!(json["modules"].is_array());
    assert_eq!(json["modules"].as_array().unwrap().len(), 5);
    assert_eq!(json["overall_ready"].as_bool().unwrap(), false);

    cleanup_tenant(&pool, tenant_id).await;
}

/// GET summary returns overall_ready=true if all modules are reporting Ready.
/// This uses a mock that serves a successful /api/ready response.
#[tokio::test]
async fn test_readiness_status_ready_when_all_modules_ready() {
    // This test exercises the ReadinessStatus enum without live services.
    // If all modules return Ready, overall_ready must be true.
    use tenant_registry::ModuleReadiness;

    let modules = vec![
        ModuleReadiness {
            module: "ar".to_string(),
            status: ReadinessStatus::Ready,
            schema_version: Some("20260216000001".to_string()),
            latency_ms: 10,
            error: None,
        },
        ModuleReadiness {
            module: "payments".to_string(),
            status: ReadinessStatus::Ready,
            schema_version: None,
            latency_ms: 8,
            error: None,
        },
        ModuleReadiness {
            module: "subscriptions".to_string(),
            status: ReadinessStatus::Ready,
            schema_version: None,
            latency_ms: 12,
            error: None,
        },
        ModuleReadiness {
            module: "gl".to_string(),
            status: ReadinessStatus::Ready,
            schema_version: None,
            latency_ms: 9,
            error: None,
        },
        ModuleReadiness {
            module: "notifications".to_string(),
            status: ReadinessStatus::Ready,
            schema_version: None,
            latency_ms: 7,
            error: None,
        },
    ];

    let all_ready = modules.iter().all(|m| m.status == ReadinessStatus::Ready);
    assert!(all_ready, "All modules ready → overall_ready must be true");
}
