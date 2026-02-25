//! Integration tests for GET /api/reporting/kpis endpoint (bd-22lo).
//!
//! Tests verify KPI computation through the full HTTP stack with real DB.

mod helpers;

use axum::{body::Body, http::Request};
use helpers::{
    body_json, build_test_app, seed_ap_aging, seed_ar_aging, seed_cashflow,
    seed_kpi_cache, seed_trial_balance, setup_db, unique_tenant,
};
use serial_test::serial;
use tower::ServiceExt;
use uuid::Uuid;

// ── Happy path ──────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kpis_with_ar_data_returns_outstanding() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    let cust = format!("cust-{}", Uuid::new_v4().simple());
    seed_ar_aging(&pool, &tid_str, "2026-01-31", &cust).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/reporting/kpis?as_of=2026-01-31"
                ))
                .header("x-tenant-id", tid_str.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let ar = json["ar_total_outstanding"]["USD"].as_i64().unwrap_or(0);
    assert!(ar > 0, "AR outstanding should be non-zero, got {ar}");
}

#[tokio::test]
#[serial]
async fn kpis_with_ap_data_returns_outstanding() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    let vendor = format!("vendor-{}", Uuid::new_v4().simple());
    seed_ap_aging(&pool, &tid_str, "2026-01-31", &vendor).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let ap = json["ap_total_outstanding"]["USD"].as_i64().unwrap_or(0);
    assert!(ap > 0, "AP outstanding should be non-zero, got {ap}");
}

#[tokio::test]
#[serial]
async fn kpis_with_mrr_from_kpi_cache() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    seed_kpi_cache(&pool, &tid_str, "2026-01-31", "mrr", "USD", 250_000).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    assert_eq!(json["mrr"]["USD"].as_i64().unwrap(), 250_000);
}

#[tokio::test]
#[serial]
async fn kpis_with_inventory_value_from_kpi_cache() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    seed_kpi_cache(&pool, &tid_str, "2026-01-31", "inventory_value", "USD", 999_000).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    assert_eq!(json["inventory_value"]["USD"].as_i64().unwrap(), 999_000);
}

#[tokio::test]
#[serial]
async fn kpis_burn_ytd_from_expense_accounts() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    // Expense accounts 5xxx-6xxx
    seed_trial_balance(&pool, &tid_str, "2026-01-31", "5000", "Wages", "USD", 60_000, 0).await;
    seed_trial_balance(&pool, &tid_str, "2026-01-31", "6000", "Rent", "USD", 40_000, 0).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let burn = json["burn_ytd"]["USD"].as_i64().unwrap_or(0);
    assert_eq!(burn, 100_000, "Burn should sum expense accounts");
}

#[tokio::test]
#[serial]
async fn kpis_cash_collected_ytd_from_cashflow() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    seed_cashflow(&pool, &tid_str, "2026-01-01", "2026-01-31").await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let cash = json["cash_collected_ytd"]["USD"].as_i64().unwrap_or(0);
    assert!(cash > 0, "Cash collected YTD should be non-zero");
}

// ── Multi-currency ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kpis_multi_currency_ar() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    // Seed AR in USD and EUR
    seed_ar_aging(&pool, &tid_str, "2026-01-31", "cust-usd").await;
    // Seed EUR manually (different currency)
    sqlx::query(
        r#"INSERT INTO rpt_ar_aging_cache
            (tenant_id, as_of, customer_id, currency, current_minor,
             bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
             bucket_over_90_minor, total_minor)
        VALUES ($1, '2026-01-31'::DATE, 'cust-eur', 'EUR', 7000, 3000, 0, 0, 0, 10000)
        ON CONFLICT (tenant_id, as_of, customer_id, currency) DO NOTHING"#,
    )
    .bind(&tid_str)
    .execute(&pool)
    .await
    .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    assert!(json["ar_total_outstanding"]["USD"].as_i64().unwrap() > 0);
    assert_eq!(json["ar_total_outstanding"]["EUR"].as_i64().unwrap(), 10000);
}

// ── Unknown tenant ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kpis_unknown_tenant_returns_empty_maps() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    assert_eq!(json["ar_total_outstanding"], serde_json::json!({}));
    assert_eq!(json["ap_total_outstanding"], serde_json::json!({}));
    assert_eq!(json["mrr"], serde_json::json!({}));
    assert_eq!(json["burn_ytd"], serde_json::json!({}));
}

// ── Validation errors ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kpis_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis")
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
async fn kpis_no_auth_returns_401() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    // No x-tenant-id header → no VerifiedClaims → 401
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

// ── Tenant isolation ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn kpis_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let app = build_test_app(pool.clone());

    seed_ar_aging(&pool, &tid_a.to_string(), "2026-01-31", "cust-x").await;

    // Tenant A sees AR data
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json_a = body_json(resp_a).await;
    assert!(json_a["ar_total_outstanding"]["USD"].as_i64().unwrap() > 0);

    // Tenant B sees nothing
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/kpis?as_of=2026-01-31")
                .header("x-tenant-id", tid_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json_b = body_json(resp_b).await;
    assert_eq!(json_b["ar_total_outstanding"], serde_json::json!({}));
}
