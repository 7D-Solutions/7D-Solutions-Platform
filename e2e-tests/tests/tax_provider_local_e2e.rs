//! E2E Test: Local Tax Provider + Cached Quote Storage (bd-29j)
//!
//! **Coverage:**
//! 1. POST /api/ar/tax/quote → LocalTaxProvider calculates deterministic tax for CA
//! 2. Repeat POST with identical payload → returns cached=true, same total
//! 3. GET /api/ar/tax/quote?app_id=...&invoice_id=... → retrieves cached quote
//! 4. POST with changed line items → cache miss, new quote
//! 5. POST with empty line items → 400 rejection
//! 6. DB verification: ar_tax_quote_cache row exists with correct fields
//! 7. Multi-line invoice → per-line tax breakdown is correct
//! 8. Non-US address → 0% tax (exempt)
//!
//! **Pattern:** In-process Axum router via tower::ServiceExt::oneshot.
//! No Docker, no mocks — uses live AR database pool.
//!
//! Run with: cargo test -p e2e-tests tax_provider_local_e2e -- --nocapture

mod common;

use axum::{body::Body, http::Request};
use chrono::Utc;
use common::get_ar_pool;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Types (mirror HTTP contract)
// ============================================================================

#[derive(Debug, Serialize)]
struct TaxQuoteHttpRequest {
    app_id: String,
    invoice_id: String,
    customer_id: String,
    ship_to: TaxAddress,
    ship_from: TaxAddress,
    line_items: Vec<TaxLineItem>,
    currency: String,
    invoice_date: String,
    correlation_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct TaxAddress {
    line1: String,
    line2: Option<String>,
    city: String,
    state: String,
    postal_code: String,
    country: String,
}

#[derive(Debug, Serialize, Clone)]
struct TaxLineItem {
    line_id: String,
    description: String,
    amount_minor: i64,
    currency: String,
    tax_code: Option<String>,
    quantity: f64,
}

#[derive(Debug, Deserialize)]
struct TaxQuoteHttpResponse {
    total_tax_minor: i64,
    tax_by_line: Vec<TaxByLineHttp>,
    provider_quote_ref: String,
    provider: String,
    cached: bool,
    quoted_at: String,
}

#[derive(Debug, Deserialize)]
struct TaxByLineHttp {
    line_id: String,
    tax_minor: i64,
    rate: f64,
    jurisdiction: String,
    tax_type: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

// ============================================================================
// Helpers
// ============================================================================

fn build_tax_router(pool: PgPool) -> axum::Router {
    ar_rs::routes::tax::tax_router(pool)
}

fn ca_address() -> TaxAddress {
    TaxAddress {
        line1: "100 Market St".to_string(),
        line2: None,
        city: "San Francisco".to_string(),
        state: "CA".to_string(),
        postal_code: "94105".to_string(),
        country: "US".to_string(),
    }
}

fn ny_address() -> TaxAddress {
    TaxAddress {
        line1: "350 Fifth Ave".to_string(),
        line2: None,
        city: "New York".to_string(),
        state: "NY".to_string(),
        postal_code: "10118".to_string(),
        country: "US".to_string(),
    }
}

fn uk_address() -> TaxAddress {
    TaxAddress {
        line1: "10 Downing St".to_string(),
        line2: None,
        city: "London".to_string(),
        state: "LDN".to_string(),
        postal_code: "SW1A 2AA".to_string(),
        country: "GB".to_string(),
    }
}

fn saas_line(id: &str, amount: i64) -> TaxLineItem {
    TaxLineItem {
        line_id: id.to_string(),
        description: "SaaS subscription".to_string(),
        amount_minor: amount,
        currency: "usd".to_string(),
        tax_code: Some("SW050000".to_string()),
        quantity: 1.0,
    }
}

async fn run_tax_migration(pool: &PgPool) {
    let migration_sql = include_str!(
        "../../modules/ar/db/migrations/20260217000007_create_tax_quote_cache.sql"
    );
    // Tolerate concurrent CREATE TABLE from parallel tests (pg_type duplicate key)
    match sqlx::raw_sql(migration_sql).execute(pool).await {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("pg_type_typname_nsp_index") || msg.contains("already exists") {
                // Table already created by a concurrent test — safe to proceed
            } else {
                panic!("Failed to run tax_quote_cache migration: {}", e);
            }
        }
    }
}

async fn cleanup_tax_cache(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_tax_quote_cache WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn post_tax_quote(
    app: axum::Router,
    body: &TaxQuoteHttpRequest,
) -> (u16, String) {
    let json = serde_json::to_string(body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/ar/tax/quote")
        .header("content-type", "application/json")
        .body(Body::from(json))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn get_cached_quote(
    app: axum::Router,
    app_id: &str,
    invoice_id: &str,
) -> (u16, String) {
    let uri = format!(
        "/api/ar/tax/quote?app_id={}&invoice_id={}",
        app_id, invoice_id
    );
    let request = Request::builder()
        .method("GET")
        .uri(&uri)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_tax_quote_california_rate() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-ca-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 10000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: Some(format!("corr-{}", Uuid::new_v4())),
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200, "Expected 200, got {}: {}", status, resp_body);

    let resp: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();

    // CA rate = 8.5% → 10000 * 0.085 = 850
    assert_eq!(resp.total_tax_minor, 850, "CA tax should be 850 on $100");
    assert_eq!(resp.tax_by_line.len(), 1);
    assert_eq!(resp.tax_by_line[0].line_id, "line-1");
    assert_eq!(resp.tax_by_line[0].tax_minor, 850);
    assert!((resp.tax_by_line[0].rate - 0.085).abs() < 0.001);
    assert_eq!(resp.tax_by_line[0].jurisdiction, "California State Tax");
    assert_eq!(resp.tax_by_line[0].tax_type, "sales_tax");
    assert_eq!(resp.provider, "local");
    assert!(!resp.cached, "First call should not be cached");
    assert!(resp.provider_quote_ref.starts_with("local-quote-"));

    println!("PASS: California tax quote = {} (8.5%)", resp.total_tax_minor);

    cleanup_tax_cache(&pool, &tenant).await;
}

#[tokio::test]
async fn test_tax_quote_cache_hit_deterministic() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-ch-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        ship_to: ny_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 20000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: Some(format!("corr-{}", Uuid::new_v4())),
    };

    // First call: cache MISS → provider called
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200, "cache_hit first POST failed: {}", resp_body);
    let resp1: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(!resp1.cached, "First call should be cache miss");
    let quote_ref_1 = resp1.provider_quote_ref.clone();

    // NY rate = 8% → 20000 * 0.08 = 1600
    assert_eq!(resp1.total_tax_minor, 1600);

    // Second call: cache HIT → same total, cached=true
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200);
    let resp2: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(resp2.cached, "Second call should be cache hit");

    // Determinism: same total_tax_minor
    assert_eq!(
        resp2.total_tax_minor, resp1.total_tax_minor,
        "Cached response must return identical total"
    );

    // Same provider_quote_ref (from cache)
    assert_eq!(
        resp2.provider_quote_ref, quote_ref_1,
        "Cached response must return same quote ref"
    );

    println!(
        "PASS: Cache hit deterministic — both calls returned {}",
        resp1.total_tax_minor
    );

    cleanup_tax_cache(&pool, &tenant).await;
}

#[tokio::test]
async fn test_tax_quote_cache_lookup_get() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-gt-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    // First: POST to create a cached quote
    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 10000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, _) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200);

    // Then: GET the cached quote
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = get_cached_quote(app, &tenant, &invoice_id).await;
    assert_eq!(status, 200, "GET should return 200: {}", resp_body);

    let resp: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(resp.cached, "GET always returns cached=true");
    assert_eq!(resp.total_tax_minor, 850);
    assert_eq!(resp.provider, "local");

    println!("PASS: GET /api/ar/tax/quote returned cached quote = {}", resp.total_tax_minor);

    // GET for non-existent invoice → 404
    let app = build_tax_router(pool.clone());
    let (status, _) = get_cached_quote(app, &tenant, "nonexistent-inv").await;
    assert_eq!(status, 404, "Non-existent invoice should return 404");

    println!("PASS: GET for non-existent invoice returned 404");

    cleanup_tax_cache(&pool, &tenant).await;
}

#[tokio::test]
async fn test_tax_quote_changed_request_new_calculation() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-cg-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    // First quote: 10000 amount
    let body1 = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 10000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body1).await;
    assert_eq!(status, 200, "changed_req first POST failed: {}", resp_body);
    let resp1: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(resp1.total_tax_minor, 850);

    // Second quote: SAME invoice_id but DIFFERENT amount → different hash → new calculation
    let body2 = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 20000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body2).await;
    assert_eq!(status, 200);
    let resp2: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(resp2.total_tax_minor, 1700, "Changed amount should yield new tax");
    assert!(!resp2.cached, "Changed request hash should be cache miss");

    println!("PASS: Changed line items → new calculation ({} → {})", resp1.total_tax_minor, resp2.total_tax_minor);

    // Both cache rows should exist
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_tax_quote_cache WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 2, "Two cache rows should exist for different request hashes");

    println!("PASS: DB has {} cache rows for the invoice", count);

    cleanup_tax_cache(&pool, &tenant).await;
}

#[tokio::test]
async fn test_tax_quote_empty_lines_rejected() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-em-{}", Uuid::new_v4());

    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body).await;
    assert_eq!(status, 400, "Empty line items should be rejected: {}", resp_body);

    let err: ErrorBody = serde_json::from_str(&resp_body).unwrap();
    assert!(
        err.error.contains("No line items"),
        "Error should mention line items: {}",
        err.error
    );

    println!("PASS: Empty line items rejected with 400");
}

#[tokio::test]
async fn test_tax_quote_db_cache_row_correct() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-db-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 10000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, _) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200);

    // Verify DB row
    let row = sqlx::query(
        r#"
        SELECT app_id, invoice_id, provider, provider_quote_ref,
               total_tax_minor, request_hash, tax_by_line, response_json
        FROM ar_tax_quote_cache
        WHERE app_id = $1 AND invoice_id = $2
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .expect("Cache row not found");

    let db_app_id: String = row.get("app_id");
    let db_invoice_id: String = row.get("invoice_id");
    let db_provider: String = row.get("provider");
    let db_quote_ref: String = row.get("provider_quote_ref");
    let db_total: i64 = row.get("total_tax_minor");
    let db_hash: String = row.get("request_hash");
    let db_tax_by_line: serde_json::Value = row.get("tax_by_line");
    let db_response: serde_json::Value = row.get("response_json");

    assert_eq!(db_app_id, tenant);
    assert_eq!(db_invoice_id, invoice_id);
    assert_eq!(db_provider, "local");
    assert!(db_quote_ref.starts_with("local-quote-"));
    assert_eq!(db_total, 850);
    assert_eq!(db_hash.len(), 64, "Request hash should be SHA-256 hex");

    // tax_by_line should be a JSON array with 1 element
    let lines = db_tax_by_line.as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["line_id"], "line-1");
    assert_eq!(lines[0]["tax_minor"], 850);

    // response_json should have the full response
    assert_eq!(db_response["total_tax_minor"], 850);

    println!("PASS: DB cache row has correct app_id={}, invoice_id={}, total={}, hash={}...",
        db_app_id, db_invoice_id, db_total, &db_hash[..8]);

    cleanup_tax_cache(&pool, &tenant).await;
}

#[tokio::test]
async fn test_tax_quote_multi_line_breakdown() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-ml-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![
            saas_line("line-1", 10000),
            TaxLineItem {
                line_id: "line-2".to_string(),
                description: "Storage addon".to_string(),
                amount_minor: 5000,
                currency: "usd".to_string(),
                tax_code: None,
                quantity: 1.0,
            },
            TaxLineItem {
                line_id: "line-3".to_string(),
                description: "API calls overage".to_string(),
                amount_minor: 3000,
                currency: "usd".to_string(),
                tax_code: None,
                quantity: 100.0,
            },
        ],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200, "multi_line POST failed: {}", resp_body);
    let resp: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();

    // CA 8.5%: 10000*0.085=850, 5000*0.085=425, 3000*0.085=255 → total 1530
    assert_eq!(resp.tax_by_line.len(), 3);
    assert_eq!(resp.tax_by_line[0].tax_minor, 850);
    assert_eq!(resp.tax_by_line[1].tax_minor, 425);
    assert_eq!(resp.tax_by_line[2].tax_minor, 255);
    assert_eq!(resp.total_tax_minor, 1530);

    println!(
        "PASS: Multi-line tax = {} (850 + 425 + 255)",
        resp.total_tax_minor
    );

    cleanup_tax_cache(&pool, &tenant).await;
}

#[tokio::test]
async fn test_tax_quote_non_us_exempt() {
    let pool = get_ar_pool().await;
    run_tax_migration(&pool).await;

    let tenant = format!("tx-uk-{}", Uuid::new_v4());
    cleanup_tax_cache(&pool, &tenant).await;

    let body = TaxQuoteHttpRequest {
        app_id: tenant.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: "cust-uk".to_string(),
        ship_to: uk_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 10000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_tax_quote(app, &body).await;
    assert_eq!(status, 200);
    let resp: TaxQuoteHttpResponse = serde_json::from_str(&resp_body).unwrap();

    assert_eq!(resp.total_tax_minor, 0, "Non-US should be tax exempt");
    assert_eq!(resp.tax_by_line[0].rate, 0.0);

    println!("PASS: Non-US address → $0 tax (exempt)");

    cleanup_tax_cache(&pool, &tenant).await;
}
