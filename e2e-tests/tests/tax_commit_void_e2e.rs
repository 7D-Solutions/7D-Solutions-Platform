//! E2E Test: Tax Commit/Void Lifecycle (bd-3fy)
//!
//! **Coverage:**
//! 1. Quote → Commit: tax committed on finalize, commit ref persisted
//! 2. Idempotent commit: second commit returns same ref, no double-commit
//! 3. Commit → Void: voided on refund/cancellation, void reason persisted
//! 4. Idempotent void: second void returns same result, no double-void
//! 5. Void without commit: returns 404
//! 6. Commit without quote: returns 404
//! 7. Event outbox: tax.committed and tax.voided events emitted
//! 8. DB verification: ar_tax_commits row has correct status transitions
//!
//! **Pattern:** In-process Axum router via tower::ServiceExt::oneshot.
//! No Docker, no mocks — uses live AR database pool.
//!
//! Run with: cargo test -p e2e-tests tax_commit_void_e2e -- --nocapture

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
    provider_quote_ref: String,
    cached: bool,
}

#[derive(Debug, Serialize)]
struct CommitTaxHttpRequest {
    app_id: String,
    invoice_id: String,
    customer_id: String,
    correlation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommitTaxHttpResponse {
    provider_commit_ref: String,
    provider_quote_ref: String,
    total_tax_minor: i64,
    currency: String,
    already_committed: bool,
}

#[derive(Debug, Serialize)]
struct VoidTaxHttpRequest {
    app_id: String,
    invoice_id: String,
    void_reason: String,
    correlation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VoidTaxHttpResponse {
    provider_commit_ref: String,
    total_tax_minor: i64,
    already_voided: bool,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

// ============================================================================
// Helpers
// ============================================================================

fn build_tax_router(pool: PgPool) -> axum::Router {
    common::with_test_jwt_layer(ar_rs::http::tax::tax_router(pool))
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

async fn run_migrations(pool: &PgPool) {
    let quote_cache_sql =
        include_str!("../../modules/ar/db/migrations/20260217000007_create_tax_quote_cache.sql");
    match sqlx::raw_sql(quote_cache_sql).execute(pool).await {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("already exists") && !msg.contains("pg_type_typname_nsp_index") {
                panic!("Failed to run tax_quote_cache migration: {}", e);
            }
        }
    }

    let tax_commits_sql =
        include_str!("../../modules/ar/db/migrations/20260217000011_create_tax_commits.sql");
    match sqlx::raw_sql(tax_commits_sql).execute(pool).await {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("already exists") && !msg.contains("pg_type_typname_nsp_index") {
                panic!("Failed to run tax_commits migration: {}", e);
            }
        }
    }

    // Ensure events_outbox table exists (needed for event emission)
    let outbox_sql =
        include_str!("../../modules/ar/db/migrations/20260211000001_create_events_outbox.sql");
    match sqlx::raw_sql(outbox_sql).execute(pool).await {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("already exists") && !msg.contains("pg_type_typname_nsp_index") {
                panic!("Failed to run events_outbox migration: {}", e);
            }
        }
    }

    // Ensure envelope metadata columns exist
    let envelope_sql = include_str!(
        "../../modules/ar/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql"
    );
    match sqlx::raw_sql(envelope_sql).execute(pool).await {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("already exists") && !msg.contains("pg_type_typname_nsp_index") {
                // Column-add migrations commonly fail with "already exists" — that's fine
            }
        }
    }
}

async fn cleanup(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_tax_commits WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_tax_quote_cache WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn post_json<T: Serialize>(
    app: axum::Router,
    uri: &str,
    body: &T,
    tenant_id: &str,
) -> (u16, String) {
    let jwt = common::sign_test_jwt(tenant_id, &["ar.mutate", "ar.read"]);
    let json = serde_json::to_string(body).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", jwt))
        .body(Body::from(json))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Helper: quote tax for an invoice (setup step)
async fn quote_tax(pool: &PgPool, app_id: &str, invoice_id: &str) -> TaxQuoteHttpResponse {
    let body = TaxQuoteHttpRequest {
        app_id: app_id.to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: "cust-1".to_string(),
        ship_to: ca_address(),
        ship_from: ca_address(),
        line_items: vec![saas_line("line-1", 10000)],
        currency: "usd".to_string(),
        invoice_date: Utc::now().to_rfc3339(),
        correlation_id: Some(format!("corr-{}", Uuid::new_v4())),
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/quote", &body, app_id).await;
    assert_eq!(status, 200, "Quote failed: {}", resp_body);
    serde_json::from_str(&resp_body).unwrap()
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Quote → Commit lifecycle
#[tokio::test]
async fn test_tax_commit_after_quote() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-cm-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    // Step 1: Quote tax (creates cached quote)
    let quote_resp = quote_tax(&pool, &tenant, &invoice_id).await;
    assert_eq!(quote_resp.total_tax_minor, 850, "CA 8.5% on $100 = $8.50");
    println!("PASS: Tax quoted — total={}", quote_resp.total_tax_minor);

    // Step 2: Commit tax
    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: Some(format!("corr-commit-{}", Uuid::new_v4())),
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(status, 200, "Commit failed: {}", resp_body);

    let commit_resp: CommitTaxHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(
        !commit_resp.already_committed,
        "First commit should not be already_committed"
    );
    assert!(commit_resp.provider_commit_ref.starts_with("local-commit-"));
    assert_eq!(commit_resp.total_tax_minor, 850);
    assert_eq!(commit_resp.currency, "usd");

    println!(
        "PASS: Tax committed — ref={}, total={}",
        commit_resp.provider_commit_ref, commit_resp.total_tax_minor
    );

    // Verify DB row
    let row = sqlx::query(
        "SELECT status, provider_commit_ref, total_tax_minor FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2"
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .expect("ar_tax_commits row not found");

    let db_status: String = row.get("status");
    let db_ref: String = row.get("provider_commit_ref");
    let db_total: i64 = row.get("total_tax_minor");
    assert_eq!(db_status, "committed");
    assert_eq!(db_ref, commit_resp.provider_commit_ref);
    assert_eq!(db_total, 850);

    println!(
        "PASS: DB row verified — status={}, ref={}",
        db_status, db_ref
    );

    cleanup(&pool, &tenant).await;
}

/// Test 2: Idempotent commit — second call returns same ref, no double-commit
#[tokio::test]
async fn test_tax_commit_idempotent() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-ci-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    // Quote + first commit
    quote_tax(&pool, &tenant, &invoice_id).await;

    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: None,
    };

    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(status, 200);
    let resp1: CommitTaxHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(!resp1.already_committed);

    // Second commit — should be idempotent
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(
        status, 200,
        "Idempotent commit should return 200: {}",
        resp_body
    );
    let resp2: CommitTaxHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(
        resp2.already_committed,
        "Second commit must report already_committed"
    );
    assert_eq!(resp2.total_tax_minor, resp1.total_tax_minor);

    // Verify only ONE commit row exists
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Exactly one commit row must exist");

    println!("PASS: Idempotent commit — only 1 row, second call returned already_committed=true");

    cleanup(&pool, &tenant).await;
}

/// Test 3: Quote → Commit → Void lifecycle
#[tokio::test]
async fn test_tax_commit_then_void() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-cv-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    // Quote + Commit
    quote_tax(&pool, &tenant, &invoice_id).await;

    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, _) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(status, 200);

    // Void
    let void_body = VoidTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        void_reason: "full_refund".to_string(),
        correlation_id: Some(format!("corr-void-{}", Uuid::new_v4())),
    };
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/void", &void_body, &tenant).await;
    assert_eq!(status, 200, "Void failed: {}", resp_body);

    let void_resp: VoidTaxHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(
        !void_resp.already_voided,
        "First void should not be already_voided"
    );
    assert!(void_resp.provider_commit_ref.starts_with("local-commit-"));
    assert_eq!(void_resp.total_tax_minor, 850);

    println!(
        "PASS: Tax voided — ref={}, total={}",
        void_resp.provider_commit_ref, void_resp.total_tax_minor
    );

    // Verify DB row
    let row = sqlx::query(
        "SELECT status, void_reason, voided_at FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2"
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let db_status: String = row.get("status");
    let db_reason: Option<String> = row.get("void_reason");
    assert_eq!(db_status, "voided");
    assert_eq!(db_reason.as_deref(), Some("full_refund"));
    assert!(row
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("voided_at")
        .is_some());

    println!("PASS: DB row verified — status=voided, reason=full_refund");

    cleanup(&pool, &tenant).await;
}

/// Test 4: Idempotent void — second call returns already_voided
#[tokio::test]
async fn test_tax_void_idempotent() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-vi-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    // Quote + Commit
    quote_tax(&pool, &tenant, &invoice_id).await;
    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, _) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(status, 200);

    // First void
    let void_body = VoidTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        void_reason: "invoice_cancelled".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/void", &void_body, &tenant).await;
    assert_eq!(status, 200);
    let resp1: VoidTaxHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(!resp1.already_voided);

    // Second void — idempotent
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/void", &void_body, &tenant).await;
    assert_eq!(
        status, 200,
        "Idempotent void should return 200: {}",
        resp_body
    );
    let resp2: VoidTaxHttpResponse = serde_json::from_str(&resp_body).unwrap();
    assert!(
        resp2.already_voided,
        "Second void must report already_voided"
    );

    println!("PASS: Idempotent void — second call returned already_voided=true");

    cleanup(&pool, &tenant).await;
}

/// Test 5: Void without prior commit returns 404
#[tokio::test]
async fn test_tax_void_without_commit_rejected() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-vn-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let void_body = VoidTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        void_reason: "full_refund".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/void", &void_body, &tenant).await;
    assert_eq!(
        status, 404,
        "Void without commit should return 404: {}",
        resp_body
    );

    let err: ErrorBody = serde_json::from_str(&resp_body).unwrap();
    assert!(
        err.error.contains("No committed tax"),
        "Error should mention no committed tax: {}",
        err.error
    );

    println!("PASS: Void without commit rejected with 404");

    cleanup(&pool, &tenant).await;
}

/// Test 6: Commit without prior quote returns 404
#[tokio::test]
async fn test_tax_commit_without_quote_rejected() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-cn-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(
        status, 404,
        "Commit without quote should return 404: {}",
        resp_body
    );

    let err: ErrorBody = serde_json::from_str(&resp_body).unwrap();
    assert!(
        err.error.contains("No cached tax quote"),
        "Error should mention no quote: {}",
        err.error
    );

    println!("PASS: Commit without quote rejected with 404");

    cleanup(&pool, &tenant).await;
}

/// Test 7: tax.committed and tax.voided events emitted to outbox
#[tokio::test]
async fn test_tax_events_emitted_to_outbox() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-ev-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    // Quote + Commit
    quote_tax(&pool, &tenant, &invoice_id).await;
    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, _) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(status, 200);

    // Check tax.committed event in outbox
    let committed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'tax.committed'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        committed_count, 1,
        "Exactly one tax.committed event expected"
    );

    // Verify committed event payload
    let committed_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE tenant_id = $1 AND event_type = 'tax.committed'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(committed_payload["payload"]["invoice_id"], invoice_id);
    assert_eq!(committed_payload["payload"]["total_tax_minor"], 850);

    println!("PASS: tax.committed event found in outbox with correct payload");

    // Void
    let void_body = VoidTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        void_reason: "full_refund".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, _) = post_json(app, "/api/ar/tax/void", &void_body, &tenant).await;
    assert_eq!(status, 200);

    // Check tax.voided event in outbox
    let voided_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'tax.voided'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(voided_count, 1, "Exactly one tax.voided event expected");

    // Verify voided event payload
    let voided_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE tenant_id = $1 AND event_type = 'tax.voided'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(voided_payload["payload"]["invoice_id"], invoice_id);
    assert_eq!(voided_payload["payload"]["void_reason"], "full_refund");
    assert_eq!(voided_payload["payload"]["total_tax_minor"], 850);

    println!("PASS: tax.voided event found in outbox with correct payload");

    cleanup(&pool, &tenant).await;
}

/// Test 8: Full lifecycle DB state transitions
#[tokio::test]
async fn test_tax_commit_void_db_state_transitions() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;

    let tenant = format!("tx-st-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    // Step 1: No row initially
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "No commit row before quote");

    // Step 2: Quote (still no commit row)
    quote_tax(&pool, &tenant, &invoice_id).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "No commit row after quote (not committed yet)");

    // Step 3: Commit
    let commit_body = CommitTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: "cust-1".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, _) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(status, 200);

    let row = sqlx::query(
        "SELECT status, committed_at, voided_at, void_reason FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2"
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "committed");
    assert!(row
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("committed_at")
        .is_some());
    assert!(row
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("voided_at")
        .is_none());
    assert!(row.get::<Option<String>, _>("void_reason").is_none());

    println!("PASS: After commit — status=committed, committed_at set, voided_at null");

    // Step 4: Void
    let void_body = VoidTaxHttpRequest {
        app_id: tenant.clone(),
        invoice_id: invoice_id.clone(),
        void_reason: "write_off".to_string(),
        correlation_id: None,
    };
    let app = build_tax_router(pool.clone());
    let (status, _) = post_json(app, "/api/ar/tax/void", &void_body, &tenant).await;
    assert_eq!(status, 200);

    let row = sqlx::query(
        "SELECT status, committed_at, voided_at, void_reason FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2"
    )
    .bind(&tenant)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "voided");
    assert!(row
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("committed_at")
        .is_some());
    assert!(row
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("voided_at")
        .is_some());
    assert_eq!(
        row.get::<Option<String>, _>("void_reason").as_deref(),
        Some("write_off")
    );

    println!("PASS: After void — status=voided, voided_at set, void_reason=write_off");

    // Step 5: Cannot re-commit after void
    let app = build_tax_router(pool.clone());
    let (status, resp_body) = post_json(app, "/api/ar/tax/commit", &commit_body, &tenant).await;
    assert_eq!(
        status, 409,
        "Re-commit after void should return 409: {}",
        resp_body
    );

    println!("PASS: Re-commit after void rejected with 409");

    cleanup(&pool, &tenant).await;
}
