//! Boundary E2E Test: HTTP Close-Status Endpoint + Performance
//!
//! Tests the close-status query endpoint and performance guards:
//! - GET /api/gl/periods/{period_id}/close-status
//!
//! ## Test Coverage
//! 1. Close-status reflects sealed snapshot + hash after period close
//! 2. Performance guard ensures full workflow completes in < 1s

use chrono::{NaiveDate, Utc};
use gl_rs::contracts::period_close_v1::{
    ClosePeriodRequest, CloseStatus, CloseStatusResponse, ValidateCloseRequest,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::{get_test_pool, setup_test_account, setup_test_period};

// ============================================================================
// JWT Auth Helpers (GL service requires Bearer JWT)
// ============================================================================

#[derive(Serialize)]
struct TestJwtClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn sign_test_jwt(tenant_id: &str) -> String {
    dotenvy::dotenv().ok();
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set (loaded from .env)");
    let encoding_key =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("Invalid JWT_PRIVATE_KEY_PEM");
    let now = Utc::now();
    let claims = TestJwtClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["operator".into()],
        perms: vec!["gl.read".into(), "gl.post".into()],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    let header = Header::new(Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &encoding_key).expect("Failed to sign test JWT")
}

fn authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token)
            .parse()
            .expect("valid header value"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("Failed to build authed client")
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    // Delete journal_lines via join (no tenant_id on journal_lines)
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Helper to create a balanced journal entry for testing
async fn create_test_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    _period_id: Uuid,
    account_code_debit: &str,
    account_code_credit: &str,
    amount_minor: i64,
) -> Uuid {
    let entry_id = Uuid::new_v4();
    let posted_at = chrono::Utc::now();

    // Insert journal entry (schema: id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description)
    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id, source_subject,
            posted_at, currency, description, created_at
        )
        VALUES ($1, $2, 'test', $3, 'test.boundary', $4, 'USD', 'Test balanced entry', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(posted_at)
    .execute(pool)
    .await
    .expect("Failed to insert journal entry");

    // Insert debit line (schema: id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (
            id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor
        )
        VALUES ($1, $2, 1, $3, $4, 0)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(account_code_debit)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("Failed to insert debit line");

    // Insert credit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (
            id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor
        )
        VALUES ($1, $2, 2, $3, 0, $4)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(account_code_credit)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("Failed to insert credit line");

    entry_id
}

// ============================================================
// TEST 1: Close-Status Reflects Sealed Snapshot + Hash
// ============================================================

#[tokio::test]
#[serial]
async fn test_boundary_http_close_status_reflects_snapshot() {
    // Setup
    let pool = get_test_pool().await;
    // Use a stable UUID for tenant_id (required for JWT claims parsing)
    let tenant_id = "00000000-0000-0000-0000-000000000301";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "revenue", "credit").await;

    // Create and close accounting period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    // JWT auth
    let token = sign_test_jwt(tenant_id);
    let client = authed_client(&token);

    // Close the period first
    let close_url = format!("{}/api/gl/periods/{}/close", gl_service_url, period_id);
    let close_request = ClosePeriodRequest {
        tenant_id: tenant_id.to_string(),
        closed_by: "admin".to_string(),
        close_reason: Some("Test close".to_string()),
    };

    let close_response = client
        .post(&close_url)
        .json(&close_request)
        .send()
        .await
        .expect("Failed to close period");

    assert_eq!(
        close_response.status(),
        200,
        "Close endpoint should return 200 (got {})",
        close_response.status()
    );

    // ✅ BOUNDARY TEST: GET close-status endpoint
    let status_url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        gl_service_url, period_id, tenant_id
    );

    let response = client
        .get(&status_url)
        .send()
        .await
        .expect("Failed to make HTTP request");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from close-status endpoint"
    );

    // Assert: Response is valid JSON
    let status_response: CloseStatusResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Status reflects closed state with hash
    assert_eq!(status_response.period_id, period_id);
    assert_eq!(status_response.tenant_id, tenant_id);
    assert_eq!(status_response.period_start, "2024-02-01");
    assert_eq!(status_response.period_end, "2024-02-29");

    match status_response.close_status {
        CloseStatus::Closed {
            closed_by,
            close_hash,
            close_reason,
            ..
        } => {
            assert_eq!(closed_by, "admin");
            assert_eq!(close_reason, Some("Test close".to_string()));
            assert!(
                !close_hash.is_empty(),
                "Close hash should be present in status"
            );

            // Verify hash matches DB
            let db_hash: Option<String> =
                sqlx::query_scalar("SELECT close_hash FROM accounting_periods WHERE id = $1")
                    .bind(period_id)
                    .fetch_one(&pool)
                    .await
                    .expect("Failed to fetch hash from DB");

            assert_eq!(
                Some(close_hash),
                db_hash,
                "Close hash in status response should match DB"
            );
        }
        _ => panic!("Expected CloseStatus::Closed variant"),
    }

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 2: Performance Guard (< 1s per suite)
// ============================================================

#[tokio::test]
#[serial]
async fn test_boundary_http_period_close_performance_guard() {
    use std::time::Instant;

    // This test runs a subset of operations to ensure performance stays under 1s
    let start = Instant::now();

    let pool = get_test_pool().await;
    // Use a stable UUID for tenant_id (required for JWT claims parsing)
    let tenant_id = "00000000-0000-0000-0000-000000000302";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    setup_test_account(&pool, tenant_id, "1100", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "revenue", "credit").await;

    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    // JWT auth
    let token = sign_test_jwt(tenant_id);
    let client = authed_client(&token);

    // Validate
    let validate_url = format!(
        "{}/api/gl/periods/{}/validate-close",
        gl_service_url, period_id
    );
    client
        .post(&validate_url)
        .json(&ValidateCloseRequest {
            tenant_id: tenant_id.to_string(),
        })
        .send()
        .await
        .expect("Validate failed");

    // Close
    let close_url = format!("{}/api/gl/periods/{}/close", gl_service_url, period_id);
    client
        .post(&close_url)
        .json(&ClosePeriodRequest {
            tenant_id: tenant_id.to_string(),
            closed_by: "admin".to_string(),
            close_reason: None,
        })
        .send()
        .await
        .expect("Close failed");

    // Status
    let status_url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        gl_service_url, period_id, tenant_id
    );
    client.get(&status_url).send().await.expect("Status failed");

    cleanup_test_data(&pool, tenant_id).await;

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1000,
        "Period close workflow should complete in < 1s (actual: {}ms)",
        elapsed.as_millis()
    );
}
