//! E2E tests for API error sanitization, body size limits, and negative amount
//! rejection (bd-1otyt).
//!
//! Proves:
//! 1. DB errors in HTTP responses do NOT leak SQL keywords, table names, or
//!    constraint names — only generic error messages are returned.
//! 2. Oversized request bodies are rejected at the HTTP layer (413 or 400).
//! 3. Negative monetary amounts are rejected with validation errors in at least
//!    two money-handling endpoints (credit notes + write-offs).

mod common;

use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{Request, StatusCode};
use security::middleware::DEFAULT_BODY_LIMIT;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

// SQL/DB substrings that must NEVER appear in HTTP error responses.
// Note: "column" alone is too broad (JSON parsers use "column N" for positions),
// so we check for "column \"" which indicates a quoted DB column reference.
const FORBIDDEN_SUBSTRINGS: &[&str] = &[
    "relation \"",
    "relation \\\"",
    "sqlx",
    " SELECT ",
    " INSERT ",
    " UPDATE ",
    " DELETE ",
    "constraint \"",
    "constraint \\\"",
    "pg_catalog",
    "column \"",
    "column \\\"",
    "DETAIL:",
    "violates",
    "ar_idempotency_keys",
    "ar_invoices",
    "ar_credit_notes",
    "checkout_sessions",
    "duplicate key",
    "unique_violation",
    "foreign_key_violation",
    "does not exist",
    "DatabaseError(",
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Connect to the AR database (real, for normal tests).
async fn ar_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    match PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                Some(pool)
            } else {
                eprintln!("skipping: AR DB not responding");
                None
            }
        }
        Err(e) => {
            eprintln!("skipping: AR DB unavailable ({e})");
            None
        }
    }
}

/// Connect to a database that does NOT have AR tables (e.g. payments DB).
/// Used to trigger sqlx errors in AR middleware so we can verify sanitization.
async fn wrong_db_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("PAYMENTS_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string()
    });
    match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                Some(pool)
            } else {
                eprintln!("skipping: payments DB not responding");
                None
            }
        }
        Err(e) => {
            eprintln!("skipping: payments DB unavailable ({e})");
            None
        }
    }
}

/// Build an AR router (permissive = no JWT enforcement) with body limit.
fn build_test_ar_router(pool: sqlx::PgPool) -> axum::Router {
    ar_rs::http::ar_router_permissive(pool)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
}

/// Assert that a response body string contains none of the forbidden DB substrings.
fn assert_no_db_leaks(body: &str) {
    for keyword in FORBIDDEN_SUBSTRINGS {
        assert!(
            !body.contains(keyword),
            "HTTP response body leaked DB/SQL substring '{}'. Full body: {}",
            keyword,
            body
        );
    }
}

// ---------------------------------------------------------------------------
// Test 1: DB error sanitization — SQL details never leak in HTTP responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_api_sanitization_db_errors_not_leaked() {
    // Use a "wrong" database (payments DB) so that AR's idempotency middleware
    // fails with a sqlx error (ar_idempotency_keys table doesn't exist).
    let Some(pool) = wrong_db_pool().await else {
        return;
    };

    let app = build_test_ar_router(pool);

    // POST to a mutation endpoint with an Idempotency-Key header.
    // The idempotency middleware will try SELECT ... FROM ar_idempotency_keys
    // which doesn't exist in payments_db → sqlx error.
    // With the VerifiedClaims absent but Idempotency-Key present, the middleware
    // extracts app_id as None → returns without checking (no-op).
    // So we need a request that will hit the handler and trigger a DB error there.
    //
    // Strategy: POST a credit note request. The domain function will try to query
    // ar_credit_notes (which doesn't exist in payments_db) → sqlx error → the
    // handler wraps it in ErrorResponse. We verify no SQL leaks.
    let credit_note_body = serde_json::json!({
        "credit_note_id": Uuid::new_v4(),
        "app_id": "test-sanitization",
        "customer_id": "cust-1",
        "invoice_id": 1,
        "amount_minor": 1000,
        "currency": "usd",
        "reason": "test",
        "correlation_id": "corr-sanitize-test",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/ar/invoices/1/credit-notes")
        .header("content-type", "application/json")
        .body(Body::from(credit_note_body.to_string()))
        .expect("request");

    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_str = String::from_utf8_lossy(&body_bytes);

    // Should be an error (4xx or 5xx) since the DB doesn't have AR tables.
    assert!(
        status.is_client_error() || status.is_server_error(),
        "expected error status, got {status}"
    );

    // Core assertion: no SQL/DB details in the response body.
    assert_no_db_leaks(&body_str);

    // Also test write-off endpoint with wrong DB.
    let write_off_body = serde_json::json!({
        "write_off_id": Uuid::new_v4(),
        "app_id": "test-sanitization",
        "customer_id": "cust-1",
        "invoice_id": 1,
        "written_off_amount_minor": 500,
        "currency": "usd",
        "reason": "test",
        "correlation_id": "corr-sanitize-test-wo",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/ar/invoices/1/write-off")
        .header("content-type", "application/json")
        .body(Body::from(write_off_body.to_string()))
        .expect("request");

    let res = app.oneshot(req).await.expect("response");
    let status = res.status();
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_str = String::from_utf8_lossy(&body_bytes);

    assert!(
        status.is_client_error() || status.is_server_error(),
        "expected error status, got {status}"
    );

    assert_no_db_leaks(&body_str);
}

// ---------------------------------------------------------------------------
// Test 2: Request body size limit enforced at real HTTP layer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_api_sanitization_body_limit_enforced() {
    let Some(pool) = ar_pool().await else {
        return;
    };

    let app = build_test_ar_router(pool);

    // DEFAULT_BODY_LIMIT is 2 MB. Send 3 MB.
    let oversized = "x".repeat(3 * 1024 * 1024);

    let req = Request::builder()
        .method("POST")
        .uri("/api/ar/invoices/1/credit-notes")
        .header("content-type", "application/json")
        .body(Body::from(oversized))
        .expect("request");

    let res = app.oneshot(req).await.expect("response");
    let status = res.status();

    // axum DefaultBodyLimit returns 413 Payload Too Large
    assert!(
        status == StatusCode::PAYLOAD_TOO_LARGE || status == StatusCode::BAD_REQUEST,
        "expected 413 or 400 for oversized body, got {status}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Negative monetary amount rejected — AR credit note
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_api_sanitization_negative_amount_credit_note() {
    let Some(pool) = ar_pool().await else {
        return;
    };

    let app = build_test_ar_router(pool);

    let body = serde_json::json!({
        "credit_note_id": Uuid::new_v4(),
        "app_id": "test-negative",
        "customer_id": "cust-1",
        "invoice_id": 1,
        "amount_minor": -5000,
        "currency": "usd",
        "reason": "test negative",
        "correlation_id": "corr-neg-cn",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/ar/invoices/1/credit-notes")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");

    let res = app.oneshot(req).await.expect("response");
    let status = res.status();
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_str = String::from_utf8_lossy(&body_bytes);

    // Must be 4xx (validation error), not 5xx (DB error).
    assert!(
        status.is_client_error(),
        "negative amount should return 4xx, got {status}: {body_str}"
    );

    // Response must mention the amount being invalid, not a DB error.
    assert_no_db_leaks(&body_str);

    // Should reference the invalid amount in some form.
    let lower = body_str.to_lowercase();
    assert!(
        lower.contains("amount") || lower.contains("invalid") || lower.contains("must be"),
        "response should indicate amount validation failure: {body_str}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Negative monetary amount rejected — AR write-off
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_api_sanitization_negative_amount_write_off() {
    let Some(pool) = ar_pool().await else {
        return;
    };

    let app = build_test_ar_router(pool);

    let body = serde_json::json!({
        "write_off_id": Uuid::new_v4(),
        "app_id": "test-negative",
        "customer_id": "cust-1",
        "invoice_id": 1,
        "written_off_amount_minor": -1000,
        "currency": "usd",
        "reason": "test negative write-off",
        "correlation_id": "corr-neg-wo",
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/ar/invoices/1/write-off")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");

    let res = app.oneshot(req).await.expect("response");
    let status = res.status();
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_str = String::from_utf8_lossy(&body_bytes);

    // Must be 4xx (validation error), not 5xx (DB error).
    assert!(
        status.is_client_error(),
        "negative amount should return 4xx, got {status}: {body_str}"
    );

    assert_no_db_leaks(&body_str);

    let lower = body_str.to_lowercase();
    assert!(
        lower.contains("amount") || lower.contains("invalid") || lower.contains("must be"),
        "response should indicate amount validation failure: {body_str}"
    );
}
