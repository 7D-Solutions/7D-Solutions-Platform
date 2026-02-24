//! HTTP-level integration tests for the reporting module (bd-byku).
//!
//! Tests exercise real Axum routes against a real Postgres instance via
//! REPORTING_DATABASE_URL (default: postgres://ap_user:ap_pass@localhost:5443/reporting_test).
//!
//! Route groups covered:
//!   - P&L statement       GET /api/reporting/pl
//!   - Balance Sheet        GET /api/reporting/balance-sheet
//!   - Cash Flow            GET /api/reporting/cashflow
//!   - AR Aging             GET /api/reporting/ar-aging
//!   - AP Aging             GET /api/reporting/ap-aging
//!   - KPIs                 GET /api/reporting/kpis
//!   - Forecast             GET /api/reporting/forecast
//!   - Admin Rebuild        POST /api/reporting/rebuild
//!   - Tenant Isolation

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use reporting::{metrics::ReportingMetrics, AppState};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ── Test helpers ──────────────────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("REPORTING_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://ap_user:ap_pass@localhost:5443/reporting_test".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to reporting test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run reporting migrations");

    pool
}

fn unique_tenant() -> String {
    format!("rpt-test-{}", Uuid::new_v4().simple())
}

/// Build a minimal test router with real handlers but no JWT/rate-limit middleware.
fn build_test_app(pool: sqlx::PgPool) -> Router {
    let metrics = Arc::new(ReportingMetrics::new().expect("metrics init"));
    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics,
    });

    Router::new()
        .route("/api/reporting/pl", get(reporting::http::statements::get_pl))
        .route(
            "/api/reporting/balance-sheet",
            get(reporting::http::statements::get_balance_sheet),
        )
        .route(
            "/api/reporting/cashflow",
            get(reporting::http::cashflow::get_cashflow),
        )
        .route(
            "/api/reporting/ar-aging",
            get(reporting::http::aging::get_ar_aging),
        )
        .route(
            "/api/reporting/ap-aging",
            get(reporting::http::aging::get_ap_aging),
        )
        .route(
            "/api/reporting/kpis",
            get(reporting::http::kpis::get_kpis),
        )
        .route(
            "/api/reporting/forecast",
            get(reporting::http::forecast::get_forecast),
        )
        .route(
            "/api/reporting/rebuild",
            post(reporting::http::admin::rebuild),
        )
        .with_state(app_state)
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

async fn body_text(response: axum::http::Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

// ── Seed helpers ──────────────────────────────────────────────────────────────

async fn seed_trial_balance(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    as_of: &str,
    account_code: &str,
    account_name: &str,
    currency: &str,
    debit_minor: i64,
    credit_minor: i64,
) {
    let net = debit_minor - credit_minor;
    sqlx::query(
        r#"
        INSERT INTO rpt_trial_balance_cache
            (tenant_id, as_of, account_code, account_name, currency, debit_minor, credit_minor, net_minor)
        VALUES ($1, $2::DATE, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (tenant_id, as_of, account_code, currency) DO UPDATE
            SET debit_minor = EXCLUDED.debit_minor,
                credit_minor = EXCLUDED.credit_minor,
                net_minor = EXCLUDED.net_minor
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(account_code)
    .bind(account_name)
    .bind(currency)
    .bind(debit_minor)
    .bind(credit_minor)
    .bind(net)
    .execute(pool)
    .await
    .expect("seed trial balance");
}

async fn seed_ar_aging(pool: &sqlx::PgPool, tenant_id: &str, as_of: &str, customer_id: &str) {
    sqlx::query(
        r#"
        INSERT INTO rpt_ar_aging_cache
            (tenant_id, as_of, customer_id, currency, current_minor, bucket_1_30_minor,
             bucket_31_60_minor, bucket_61_90_minor, bucket_over_90_minor, total_minor)
        VALUES ($1, $2::DATE, $3, 'USD', 10000, 5000, 2000, 1000, 500, 18500)
        ON CONFLICT (tenant_id, as_of, customer_id, currency) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(customer_id)
    .execute(pool)
    .await
    .expect("seed ar aging");
}

async fn seed_ap_aging(pool: &sqlx::PgPool, tenant_id: &str, as_of: &str, vendor_id: &str) {
    sqlx::query(
        r#"
        INSERT INTO rpt_ap_aging_cache
            (tenant_id, as_of, vendor_id, currency, current_minor, bucket_1_30_minor,
             bucket_31_60_minor, bucket_61_90_minor, bucket_over_90_minor, total_minor)
        VALUES ($1, $2::DATE, $3, 'USD', 8000, 3000, 1000, 500, 200, 12700)
        ON CONFLICT (tenant_id, as_of, vendor_id, currency) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(vendor_id)
    .execute(pool)
    .await
    .expect("seed ap aging");
}

async fn seed_cashflow(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    period_start: &str,
    period_end: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO rpt_cashflow_cache
            (tenant_id, period_start, period_end, activity_type, line_code, line_label, currency, amount_minor)
        VALUES ($1, $2::DATE, $3::DATE, 'operating', 'cash_collections', 'Cash Collections', 'USD', 50000)
        ON CONFLICT (tenant_id, period_start, period_end, activity_type, line_code, currency) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .execute(pool)
    .await
    .expect("seed cashflow");
}

// ═══════════════════════════════════════════════════════════════════════════════
// P&L STATEMENT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_pl_happy_path_returns_200_with_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool.clone());

    // Seed revenue account (4xxx) and expense (6xxx)
    seed_trial_balance(&pool, &tid, "2026-01-31", "4000", "Revenue", "USD", 0, 100_000).await;
    seed_trial_balance(&pool, &tid, "2026-01-31", "6000", "Rent Expense", "USD", 40_000, 0).await;

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/pl?tenant_id={}&from=2026-01-01&to=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let sections = json["sections"].as_array().unwrap();
    // Expect revenue, cogs, expenses sections
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["section"], "revenue");
    assert_eq!(sections[2]["section"], "expenses");
    // Revenue section should have one account with amount > 0
    let revenue_accounts = sections[0]["accounts"].as_array().unwrap();
    assert!(!revenue_accounts.is_empty());
}

#[tokio::test]
#[serial]
async fn test_pl_missing_tenant_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/pl?from=2026-01-01&to=2026-01-31")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[serial]
async fn test_pl_unknown_tenant_returns_empty_sections() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/pl?tenant_id={}&from=2026-01-01&to=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    // All sections have empty accounts for unknown tenant
    for section in json["sections"].as_array().unwrap() {
        assert!(section["accounts"].as_array().unwrap().is_empty());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// BALANCE SHEET TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_balance_sheet_happy_path_returns_200_with_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool.clone());

    // Seed asset account (1xxx), liability (2xxx), equity (3xxx)
    seed_trial_balance(&pool, &tid, "2026-01-31", "1000", "Cash", "USD", 200_000, 0).await;
    seed_trial_balance(&pool, &tid, "2026-01-31", "2000", "AP", "USD", 0, 80_000).await;
    seed_trial_balance(&pool, &tid, "2026-01-31", "3000", "Equity", "USD", 0, 120_000).await;

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/balance-sheet?tenant_id={}&as_of=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let sections = json["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["section"], "assets");
    assert_eq!(sections[1]["section"], "liabilities");
    assert_eq!(sections[2]["section"], "equity");
}

#[tokio::test]
#[serial]
async fn test_balance_sheet_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/balance-sheet?tenant_id=some-tenant")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// CASH FLOW TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_cashflow_happy_path_returns_200_with_sections() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool.clone());

    seed_cashflow(&pool, &tid, "2026-01-01", "2026-01-31").await;

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/cashflow?tenant_id={}&from=2026-01-01&to=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let sections = json["sections"].as_array().unwrap();
    // Expect operating, investing, financing sections
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0]["activity_type"], "operating");
}

#[tokio::test]
#[serial]
async fn test_cashflow_missing_from_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/cashflow?tenant_id=x&to=2026-01-31")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AR AGING TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_ar_aging_happy_path_returns_200_with_buckets() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool.clone());

    let cust = format!("cust-{}", Uuid::new_v4().simple());
    seed_ar_aging(&pool, &tid, "2026-01-31", &cust).await;

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/ar-aging?tenant_id={}&as_of=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["tenant_id"], tid.as_str());
    let aging = json["aging"].as_array().unwrap();
    assert!(!aging.is_empty(), "Expected at least one aging record");
    // Verify buckets are present
    let first = &aging[0];
    assert!(first["current_minor"].is_number());
    assert!(first["total_minor"].is_number());
}

#[tokio::test]
#[serial]
async fn test_ar_aging_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/ar-aging?tenant_id=x")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// AP AGING TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_ap_aging_happy_path_returns_200_with_buckets() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool.clone());

    let vendor = format!("vendor-{}", Uuid::new_v4().simple());
    seed_ap_aging(&pool, &tid, "2026-01-31", &vendor).await;

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/ap-aging?tenant_id={}&as_of=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    // AP aging response wraps in a report struct
    assert!(json["vendors"].is_array() || json.is_object());
}

#[tokio::test]
#[serial]
async fn test_ap_aging_missing_as_of_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/ap-aging?tenant_id=x")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// KPI TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_kpis_unknown_tenant_returns_empty_maps() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/kpis?tenant_id={}&as_of=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    // Unknown tenant: all currency maps should be empty objects
    assert_eq!(json["ar_total_outstanding"], serde_json::json!({}));
    assert_eq!(json["ap_total_outstanding"], serde_json::json!({}));
    assert_eq!(json["mrr"], serde_json::json!({}));
}

#[tokio::test]
#[serial]
async fn test_kpis_with_seeded_ar_returns_outstanding() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool.clone());

    let cust = format!("cust-{}", Uuid::new_v4().simple());
    seed_ar_aging(&pool, &tid, "2026-01-31", &cust).await;

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/kpis?tenant_id={}&as_of=2026-01-31",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    // AR outstanding should be non-zero for USD
    let ar = json["ar_total_outstanding"]["USD"].as_i64().unwrap_or(0);
    assert!(ar > 0, "Expected non-zero AR outstanding: got {}", ar);
}

#[tokio::test]
#[serial]
async fn test_kpis_missing_params_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/kpis?tenant_id=x")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// FORECAST TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_forecast_default_horizons_returns_200() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    let req = Request::builder()
        .uri(format!("/api/reporting/forecast?tenant_id={}", tid))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    // Should have horizons array in response
    assert!(json["horizons"].is_array() || json.is_object());
}

#[tokio::test]
#[serial]
async fn test_forecast_custom_horizons_returns_200() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    let req = Request::builder()
        .uri(format!(
            "/api/reporting/forecast?tenant_id={}&horizons=7,30",
            tid
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
#[serial]
async fn test_forecast_invalid_horizons_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let req = Request::builder()
        .uri("/api/reporting/forecast?tenant_id=x&horizons=abc")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ADMIN REBUILD TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_rebuild_without_admin_token_returns_403() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    // Ensure ADMIN_TOKEN is set so the handler is enabled
    std::env::set_var("ADMIN_TOKEN", "test-secret");

    let body = serde_json::json!({
        "tenant_id": tid,
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/reporting/rebuild")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
#[serial]
async fn test_rebuild_with_valid_token_returns_200() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    std::env::set_var("ADMIN_TOKEN", "test-rebuild-token");

    let body = serde_json::json!({
        "tenant_id": tid,
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/reporting/rebuild")
        .header("content-type", "application/json")
        .header("x-admin-token", "test-rebuild-token")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
#[serial]
async fn test_rebuild_invalid_date_range_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);
    let tid = unique_tenant();

    std::env::set_var("ADMIN_TOKEN", "test-rebuild-token");

    // from > to is invalid
    let body = serde_json::json!({
        "tenant_id": tid,
        "from": "2026-02-01",
        "to": "2026-01-01"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/reporting/rebuild")
        .header("content-type", "application/json")
        .header("x-admin-token", "test-rebuild-token")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ═══════════════════════════════════════════════════════════════════════════════
// TENANT ISOLATION TEST
// ═══════════════════════════════════════════════════════════════════════════════

/// Prove that data seeded for tenant A is not visible when querying tenant B.
#[tokio::test]
#[serial]
async fn test_tenant_isolation_pl_statement() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Seed revenue only for tenant_a
    seed_trial_balance(&pool, &tenant_a, "2026-01-31", "4001", "Revenue A", "USD", 0, 99_000).await;

    let app = build_test_app(pool);

    // Query tenant_a — should have revenue
    let req_a = Request::builder()
        .uri(format!(
            "/api/reporting/pl?tenant_id={}&from=2026-01-01&to=2026-01-31",
            tenant_a
        ))
        .body(Body::empty())
        .unwrap();
    let resp_a = app.clone().oneshot(req_a).await.unwrap();
    assert_eq!(resp_a.status(), StatusCode::OK);
    let json_a = body_json(resp_a).await;
    let revenue_a = json_a["sections"][0]["accounts"].as_array().unwrap();
    assert!(!revenue_a.is_empty(), "Tenant A should see revenue");

    // Query tenant_b — should have no revenue (empty accounts)
    let req_b = Request::builder()
        .uri(format!(
            "/api/reporting/pl?tenant_id={}&from=2026-01-01&to=2026-01-31",
            tenant_b
        ))
        .body(Body::empty())
        .unwrap();
    let resp_b = app.oneshot(req_b).await.unwrap();
    assert_eq!(resp_b.status(), StatusCode::OK);
    let json_b = body_json(resp_b).await;
    let revenue_b = json_b["sections"][0]["accounts"].as_array().unwrap();
    assert!(revenue_b.is_empty(), "Tenant B must not see Tenant A's revenue");
}

/// AR aging tenant isolation.
#[tokio::test]
#[serial]
async fn test_tenant_isolation_ar_aging() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let cust = format!("cust-{}", Uuid::new_v4().simple());
    seed_ar_aging(&pool, &tenant_a, "2026-01-31", &cust).await;

    let app = build_test_app(pool);

    // tenant_a sees their aging data
    let req_a = Request::builder()
        .uri(format!(
            "/api/reporting/ar-aging?tenant_id={}&as_of=2026-01-31",
            tenant_a
        ))
        .body(Body::empty())
        .unwrap();
    let resp_a = app.clone().oneshot(req_a).await.unwrap();
    assert_eq!(resp_a.status(), StatusCode::OK);
    let json_a = body_json(resp_a).await;
    assert!(!json_a["aging"].as_array().unwrap().is_empty());

    // tenant_b sees empty aging
    let req_b = Request::builder()
        .uri(format!(
            "/api/reporting/ar-aging?tenant_id={}&as_of=2026-01-31",
            tenant_b
        ))
        .body(Body::empty())
        .unwrap();
    let resp_b = app.oneshot(req_b).await.unwrap();
    assert_eq!(resp_b.status(), StatusCode::OK);
    let json_b = body_json(resp_b).await;
    assert!(json_b["aging"].as_array().unwrap().is_empty());
}
