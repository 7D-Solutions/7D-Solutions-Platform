//! Boundary E2E Test: HTTP Validate-Close Endpoint
//!
//! Tests the validate-close pre-flight endpoint:
//! - POST /api/gl/periods/{period_id}/validate-close
//!
//! ## Test Coverage
//! 1. Successful validation on open period with balanced entries
//! 2. Validation failure on already-closed period (PERIOD_ALREADY_CLOSED error)

use chrono::{NaiveDate, Utc};
use gl_rs::contracts::period_close_v1::{ValidateCloseRequest, ValidateCloseResponse};
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

/// Helper to close a period directly via SQL (for testing already-closed scenarios)
async fn close_period_directly(pool: &PgPool, period_id: Uuid, closed_by: &str, close_hash: &str) {
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = NOW(),
            closed_by = $2,
            close_hash = $3
        WHERE id = $1
        "#,
    )
    .bind(period_id)
    .bind(closed_by)
    .bind(close_hash)
    .execute(pool)
    .await
    .expect("Failed to close period directly");
}

// ============================================================
// TEST 1: Validate-Close on Open Period (Success)
// ============================================================

#[tokio::test]
#[serial]
async fn test_boundary_http_validate_close_success() {
    // Setup
    let pool = get_test_pool().await;
    // Use a stable UUID for tenant_id (required for JWT claims parsing)
    let tenant_id = "00000000-0000-0000-0000-000000000101";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "revenue", "credit").await;

    // Create open accounting period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Create balanced journal entry (validation should pass)
    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    // JWT auth
    let token = sign_test_jwt(tenant_id);
    let client = authed_client(&token);

    // ✅ BOUNDARY TEST: POST to validate-close endpoint
    let url = format!(
        "{}/api/gl/periods/{}/validate-close",
        gl_service_url, period_id
    );

    let request_body = ValidateCloseRequest {
        tenant_id: tenant_id.to_string(),
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .expect("Failed to make HTTP request - is GL service running on port 8090?");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from validate-close endpoint"
    );

    // Assert: Response is valid JSON matching ValidateCloseResponse structure
    let validate_response: ValidateCloseResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Validation passes (can_close = true)
    assert_eq!(validate_response.period_id, period_id);
    assert_eq!(validate_response.tenant_id, tenant_id);
    assert!(
        validate_response.can_close,
        "Period should pass validation (balanced entries, no issues)"
    );
    assert!(
        validate_response.validation_report.issues.is_empty(),
        "Validation report should have no issues"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 2: Validate-Close Fails on Already-Closed Period
// ============================================================

#[tokio::test]
#[serial]
async fn test_boundary_http_validate_close_fails_on_closed_period() {
    // Setup
    let pool = get_test_pool().await;
    // Use a stable UUID for tenant_id (required for JWT claims parsing)
    let tenant_id = "00000000-0000-0000-0000-000000000102";
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

    // Close the period directly
    close_period_directly(&pool, period_id, "admin", "test-hash-12345").await;

    // JWT auth
    let token = sign_test_jwt(tenant_id);
    let client = authed_client(&token);

    // ✅ BOUNDARY TEST: POST to validate-close on closed period
    let url = format!(
        "{}/api/gl/periods/{}/validate-close",
        gl_service_url, period_id
    );

    let request_body = ValidateCloseRequest {
        tenant_id: tenant_id.to_string(),
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .expect("Failed to make HTTP request");

    // Assert: 200 OK (validation endpoint returns structured report, not error)
    assert_eq!(response.status(), 200);

    // Assert: Validation fails (can_close = false)
    let validate_response: ValidateCloseResponse =
        response.json().await.expect("Failed to parse response");

    assert_eq!(validate_response.period_id, period_id);
    assert!(
        !validate_response.can_close,
        "Should not allow closing an already-closed period"
    );
    assert!(
        !validate_response.validation_report.issues.is_empty(),
        "Should have validation issues"
    );

    // Assert: Validation report contains PERIOD_ALREADY_CLOSED error
    let has_closed_error = validate_response
        .validation_report
        .issues
        .iter()
        .any(|issue| issue.code == "PERIOD_ALREADY_CLOSED");

    assert!(
        has_closed_error,
        "Validation report should contain PERIOD_ALREADY_CLOSED error"
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
