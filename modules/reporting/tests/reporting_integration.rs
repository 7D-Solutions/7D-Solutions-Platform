//! HTTP integration tests for reporting statements, aging, and tenant isolation.
//!
//! Tests exercise real Axum routes against a real Postgres instance.
//! KPI, forecast, and admin tests are in separate files.

mod helpers;

use axum::{body::Body, http::Request};
use helpers::{
    body_json, build_test_app, seed_ap_aging, seed_ar_aging, seed_cashflow, seed_trial_balance,
    setup_db, unique_tenant,
};
use serial_test::serial;
use tower::ServiceExt;
use uuid::Uuid;

// ═══════════════════════════════════════════════════════════════════════════════
// P&L STATEMENT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn pl_happy_path_returns_200_with_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "4000",
        "Revenue",
        "USD",
        0,
        100_000,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "6000",
        "Rent Expense",
        "USD",
        40_000,
        0,
    )
    .await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/pl?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let json = body_json(resp).await;
    let sections = json["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["section"], "revenue");
    assert_eq!(sections[2]["section"], "expenses");
    let revenue = sections[0]["accounts"].as_array().unwrap();
    assert!(!revenue.is_empty());
}

#[tokio::test]
#[serial]
async fn pl_no_auth_returns_401() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/pl?from=2026-01-01&to=2026-01-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
#[serial]
async fn pl_missing_from_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/pl?to=2026-01-31")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
#[serial]
async fn pl_unknown_tenant_returns_empty_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/pl?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    for section in json["sections"].as_array().unwrap() {
        assert!(section["accounts"].as_array().unwrap().is_empty());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// BALANCE SHEET TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn balance_sheet_happy_path_returns_200_with_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        200_000,
        0,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "2000",
        "AP",
        "USD",
        0,
        80_000,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "3000",
        "Equity",
        "USD",
        0,
        120_000,
    )
    .await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/balance-sheet?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let json = body_json(resp).await;
    let sections = json["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["section"], "assets");
    assert_eq!(sections[1]["section"], "liabilities");
    assert_eq!(sections[2]["section"], "equity");
}

#[tokio::test]
#[serial]
async fn balance_sheet_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/balance-sheet")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ═══════════════════════════════════════════════════════════════════════════════
// CASH FLOW TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn cashflow_happy_path_returns_200_with_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    seed_cashflow(&pool, &tid_str, "2026-01-01", "2026-01-31").await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/cashflow?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let json = body_json(resp).await;
    let sections = json["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["activity_type"], "operating");
}

#[tokio::test]
#[serial]
async fn cashflow_missing_from_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/cashflow?to=2026-01-31")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AR AGING TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn ar_aging_happy_path_returns_200_with_buckets() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    let cust = format!("cust-{}", Uuid::new_v4().simple());
    seed_ar_aging(&pool, &tid_str, "2026-01-31", &cust).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ar-aging?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let json = body_json(resp).await;
    assert_eq!(json["tenant_id"], tid.to_string());
    let aging = json["aging"].as_array().unwrap();
    assert!(!aging.is_empty());
    assert!(aging[0]["current_minor"].is_number());
    assert!(aging[0]["total_minor"].is_number());
}

#[tokio::test]
#[serial]
async fn ar_aging_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ar-aging")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AP AGING TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn ap_aging_happy_path_returns_200() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    let vendor = format!("vendor-{}", Uuid::new_v4().simple());
    seed_ap_aging(&pool, &tid_str, "2026-01-31", &vendor).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ap-aging?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
#[serial]
async fn ap_aging_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ap-aging")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ═══════════════════════════════════════════════════════════════════════════════
// TENANT ISOLATION TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn tenant_isolation_pl_statement() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    seed_trial_balance(
        &pool,
        &tenant_a.to_string(),
        "2026-01-31",
        "4001",
        "Revenue A",
        "USD",
        0,
        99_000,
    )
    .await;

    let app = build_test_app(pool);

    // Tenant A sees revenue
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/pl?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tenant_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), 200);
    let json_a = body_json(resp_a).await;
    let revenue_a = json_a["sections"][0]["accounts"].as_array().unwrap();
    assert!(!revenue_a.is_empty(), "Tenant A should see revenue");

    // Tenant B sees nothing
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/pl?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tenant_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), 200);
    let json_b = body_json(resp_b).await;
    let revenue_b = json_b["sections"][0]["accounts"].as_array().unwrap();
    assert!(revenue_b.is_empty(), "Tenant B must not see A's revenue");
}

#[tokio::test]
#[serial]
async fn tenant_isolation_ar_aging() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let cust = format!("cust-{}", Uuid::new_v4().simple());
    seed_ar_aging(&pool, &tenant_a.to_string(), "2026-01-31", &cust).await;

    let app = build_test_app(pool);

    // Tenant A sees aging data
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ar-aging?as_of=2026-01-31")
                .header("x-tenant-id", tenant_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), 200);
    let json_a = body_json(resp_a).await;
    assert!(!json_a["aging"].as_array().unwrap().is_empty());

    // Tenant B sees empty
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ar-aging?as_of=2026-01-31")
                .header("x-tenant-id", tenant_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), 200);
    let json_b = body_json(resp_b).await;
    assert!(json_b["aging"].as_array().unwrap().is_empty());
}

#[tokio::test]
#[serial]
async fn tenant_isolation_balance_sheet() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    seed_trial_balance(
        &pool,
        &tenant_a.to_string(),
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        300_000,
        0,
    )
    .await;

    let app = build_test_app(pool);

    // Tenant A sees assets
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/balance-sheet?as_of=2026-01-31")
                .header("x-tenant-id", tenant_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), 200);
    let json_a = body_json(resp_a).await;
    let assets_a = json_a["sections"][0]["accounts"].as_array().unwrap();
    assert!(!assets_a.is_empty(), "Tenant A should see assets");

    // Tenant B sees nothing
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/balance-sheet?as_of=2026-01-31")
                .header("x-tenant-id", tenant_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), 200);
    let json_b = body_json(resp_b).await;
    let assets_b = json_b["sections"][0]["accounts"].as_array().unwrap();
    assert!(
        assets_b.is_empty(),
        "Tenant B must not see A's balance sheet"
    );
}

#[tokio::test]
#[serial]
async fn tenant_isolation_cashflow() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    seed_cashflow(&pool, &tenant_a.to_string(), "2026-01-01", "2026-01-31").await;

    let app = build_test_app(pool);

    // Tenant A sees operating lines
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/cashflow?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tenant_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), 200);
    let json_a = body_json(resp_a).await;
    let operating_a = &json_a["sections"][0];
    assert!(
        !operating_a["lines"].as_array().unwrap().is_empty(),
        "Tenant A should see cashflow lines"
    );

    // Tenant B sees empty sections
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/cashflow?from=2026-01-01&to=2026-01-31")
                .header("x-tenant-id", tenant_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), 200);
    let json_b = body_json(resp_b).await;
    let operating_b = &json_b["sections"][0];
    assert!(
        operating_b["lines"].as_array().unwrap().is_empty(),
        "Tenant B must not see A's cashflow"
    );
}

#[tokio::test]
#[serial]
async fn tenant_isolation_ap_aging() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let vendor = format!("vendor-{}", Uuid::new_v4().simple());
    seed_ap_aging(&pool, &tenant_a.to_string(), "2026-01-31", &vendor).await;

    let app = build_test_app(pool);

    // Tenant A sees AP aging
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ap-aging?as_of=2026-01-31")
                .header("x-tenant-id", tenant_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), 200);
    let json_a = body_json(resp_a).await;
    assert!(
        !json_a["vendors"].as_array().unwrap().is_empty(),
        "Tenant A should see AP aging vendors"
    );

    // Tenant B sees empty
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/ap-aging?as_of=2026-01-31")
                .header("x-tenant-id", tenant_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), 200);
    let json_b = body_json(resp_b).await;
    assert!(
        json_b["vendors"].as_array().unwrap().is_empty(),
        "Tenant B must not see A's AP aging"
    );
}
