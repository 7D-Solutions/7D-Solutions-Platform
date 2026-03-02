/// Integration tests for BFF tenant list and detail routes.
///
/// These routes are served by tenant-registry but merged into the control-plane
/// router via build_router(). Tests verify the full BFF path:
///   - GET /api/tenants (paginated list)
///   - GET /api/tenants/:tenant_id (detail with seat_limit)
///   - GET /api/tenants/:tenant_id/app-id (app_id resolution)
///   - GET /api/ttp/plans (plan catalog)
///
/// All tests run against a real Postgres database. No mocks.
use axum::http::StatusCode;
use axum_test::TestServer;
use serde_json::Value;
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
    PgPool::connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

fn build_test_server(pool: PgPool) -> TestServer {
    let app_state = Arc::new(AppState::new(pool.clone(), None));
    let summary_state = Arc::new(SummaryState::new_local(pool));
    let router = build_router(app_state, summary_state);
    TestServer::new(router).expect("build test server")
}

/// Insert a tenant with product/plan/app_id and optional entitlements.
async fn seed_tenant_full(
    pool: &PgPool,
    product_code: &str,
    plan_code: &str,
    app_id: &str,
    seat_limit: Option<i32>,
) -> Uuid {
    let tenant_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO tenants
           (tenant_id, status, environment, module_schema_versions,
            product_code, plan_code, app_id, created_at, updated_at)
           VALUES ($1, 'active', 'development', '{}'::jsonb, $2, $3, $4, NOW(), NOW())"#,
    )
    .bind(tenant_id)
    .bind(product_code)
    .bind(plan_code)
    .bind(app_id)
    .execute(pool)
    .await
    .expect("insert tenant");

    if let Some(limit) = seat_limit {
        sqlx::query(
            r#"INSERT INTO cp_entitlements
               (tenant_id, plan_code, concurrent_user_limit, effective_at, updated_at)
               VALUES ($1, $2, $3, NOW(), NOW())"#,
        )
        .bind(tenant_id)
        .bind(plan_code)
        .bind(limit)
        .execute(pool)
        .await
        .expect("insert entitlements");
    }

    tenant_id
}

async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM cp_entitlements WHERE tenant_id = $1")
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

// ============================================================================
// GET /api/tenants — Tenant List
// ============================================================================

#[tokio::test]
async fn tenant_list_returns_paginated_response() {
    let pool = test_pool().await;
    let server = build_test_server(pool);

    let resp = server.get("/api/tenants?page=1&page_size=5").await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert!(body["tenants"].is_array());
    assert!(body["total"].is_number());
    assert_eq!(body["page"], 1);
    assert_eq!(body["page_size"], 5);
}

#[tokio::test]
async fn tenant_list_includes_seeded_tenant() {
    let pool = test_pool().await;
    let app_id = format!("app-test-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant_id = seed_tenant_full(&pool, "list-test-co", "monthly", &app_id, None).await;
    let server = build_test_server(pool.clone());

    let resp = server
        .get(&format!("/api/tenants?search={tenant_id}&page_size=100"))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    let tenants = body["tenants"].as_array().unwrap();
    let found = tenants
        .iter()
        .any(|t| t["id"].as_str() == Some(&tenant_id.to_string()));
    assert!(found, "seeded tenant should appear in list");

    let entry = tenants
        .iter()
        .find(|t| t["id"].as_str() == Some(&tenant_id.to_string()))
        .unwrap();
    assert_eq!(entry["name"], "list-test-co");
    assert_eq!(entry["status"], "active");
    assert_eq!(entry["plan"], "monthly");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tenant_list_filters_by_status() {
    let pool = test_pool().await;
    let app_id = format!("app-filt-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant_id = seed_tenant_full(&pool, "filter-co", "monthly", &app_id, None).await;
    let server = build_test_server(pool.clone());

    // Filter by active — should include our tenant
    let resp = server.get("/api/tenants?status=active&page_size=100").await;
    resp.assert_status(StatusCode::OK);
    let body: Value = resp.json();
    let tenants = body["tenants"].as_array().unwrap();
    let found = tenants
        .iter()
        .any(|t| t["id"].as_str() == Some(&tenant_id.to_string()));
    assert!(
        found,
        "active tenant should appear when filtering by active"
    );

    // Filter by suspended — should NOT include our active tenant
    let resp = server
        .get("/api/tenants?status=suspended&page_size=100")
        .await;
    resp.assert_status(StatusCode::OK);
    let body: Value = resp.json();
    let tenants = body["tenants"].as_array().unwrap();
    let found = tenants
        .iter()
        .any(|t| t["id"].as_str() == Some(&tenant_id.to_string()));
    assert!(
        !found,
        "active tenant should not appear when filtering by suspended"
    );

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tenant_list_filters_by_plan() {
    let pool = test_pool().await;
    let unique_plan = format!("plan-{}", &Uuid::new_v4().to_string()[..8]);
    let app_id = format!("app-plan-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant_id = seed_tenant_full(&pool, "plan-co", &unique_plan, &app_id, None).await;
    let server = build_test_server(pool.clone());

    let resp = server
        .get(&format!("/api/tenants?plan={unique_plan}&page_size=100"))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    let tenants = body["tenants"].as_array().unwrap();
    assert!(
        tenants
            .iter()
            .all(|t| t["plan"].as_str() == Some(&unique_plan)),
        "all returned tenants should match the plan filter"
    );
    assert!(
        tenants
            .iter()
            .any(|t| t["id"].as_str() == Some(&tenant_id.to_string())),
        "our tenant should be in the filtered results"
    );

    cleanup(&pool, tenant_id).await;
}

// ============================================================================
// GET /api/tenants/:tenant_id — Tenant Detail
// ============================================================================

#[tokio::test]
async fn tenant_detail_returns_full_dto() {
    let pool = test_pool().await;
    let app_id = format!("app-det-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant_id = seed_tenant_full(&pool, "detail-corp", "annual", &app_id, Some(25)).await;
    let server = build_test_server(pool.clone());

    let resp = server.get(&format!("/api/tenants/{tenant_id}")).await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["id"], tenant_id.to_string());
    assert_eq!(body["name"], "detail-corp");
    assert_eq!(body["status"], "active");
    assert_eq!(body["plan"], "annual");
    assert_eq!(body["app_id"], app_id);
    assert_eq!(body["seat_limit"], 25);
    assert!(body["created_at"].is_string());
    assert!(body["updated_at"].is_string());

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tenant_detail_returns_null_seat_limit_without_entitlements() {
    let pool = test_pool().await;
    let app_id = format!("app-nos-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant_id = seed_tenant_full(&pool, "no-seat-co", "monthly", &app_id, None).await;
    let server = build_test_server(pool.clone());

    let resp = server.get(&format!("/api/tenants/{tenant_id}")).await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert!(
        body["seat_limit"].is_null(),
        "seat_limit should be null without entitlements"
    );

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tenant_detail_returns_404_for_nonexistent() {
    let pool = test_pool().await;
    let server = build_test_server(pool);
    let missing = Uuid::new_v4();

    let resp = server.get(&format!("/api/tenants/{missing}")).await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// GET /api/tenants/:tenant_id/app-id — App ID Resolution
// ============================================================================

#[tokio::test]
async fn app_id_resolution_returns_correct_mapping() {
    let pool = test_pool().await;
    let app_id = format!("app-res-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant_id = seed_tenant_full(&pool, "resolve-co", "monthly", &app_id, None).await;
    let server = build_test_server(pool.clone());

    let resp = server
        .get(&format!("/api/tenants/{tenant_id}/app-id"))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["app_id"], app_id);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn app_id_resolution_returns_404_for_missing_tenant() {
    let pool = test_pool().await;
    let server = build_test_server(pool);
    let missing = Uuid::new_v4();

    let resp = server.get(&format!("/api/tenants/{missing}/app-id")).await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// GET /api/ttp/plans — Plan Catalog
// ============================================================================

#[tokio::test]
async fn plans_catalog_returns_seeded_plans() {
    let pool = test_pool().await;
    let server = build_test_server(pool);

    let resp = server.get("/api/ttp/plans").await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    let plans = body["plans"].as_array().expect("plans should be an array");

    // Seeded plans: starter, professional, enterprise (id field maps to plan_code)
    let plan_ids: Vec<&str> = plans.iter().filter_map(|p| p["id"].as_str()).collect();
    assert!(
        plan_ids.contains(&"starter"),
        "starter plan should be present"
    );
    assert!(
        plan_ids.contains(&"professional"),
        "professional plan should be present"
    );
    assert!(
        plan_ids.contains(&"enterprise"),
        "enterprise plan should be present"
    );
}
