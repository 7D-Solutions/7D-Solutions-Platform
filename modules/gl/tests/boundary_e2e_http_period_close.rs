//! Boundary E2E Test: HTTP → Router → Service → DB (Period Close Workflow)
//!
//! This test validates the REAL ingress boundary for GL period close operations:
//! 1. POST /api/gl/periods/{period_id}/validate-close - Pre-flight validation
//! 2. POST /api/gl/periods/{period_id}/close - Atomic close with snapshot + hash
//! 3. GET /api/gl/periods/{period_id}/close-status - Query close status
//!
//! ## IMPORTANT: Tests are #[ignore]d until bd-3gr is complete
//! These tests require HTTP handlers from bd-3gr (HTTP Handlers: Validate-Close, Close, Close-Status).
//! Once bd-3gr is merged, remove the #[ignore] attributes to enable these tests.
//!
//! ## Dependency Chain
//! - bd-37m (this test file) depends on bd-3gr (HTTP Handlers)
//! - bd-3gr depends on bd-2jp (✓ CLOSED), bd-3sl (IN_PROGRESS), bd-1zp (OPEN)
//!
//! ## Architecture Decision
//! Per ChatGPT guidance: "E2E for microservices means crossing the ACTUAL ingress boundary."
//! Write path ingress for period close = HTTP POST/GET, so these tests hit real HTTP endpoints.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - GL HTTP server at localhost:8090
//! - PostgreSQL at localhost:5438
//!
//! ## Test Strategy
//! Tests cover all acceptance criteria from bd-37m:
//! 1. Successful validate on open period
//! 2. Successful close operation
//! 3. Idempotent close (repeated calls return same status + hash)
//! 4. Close-status reflects sealed snapshot/hash
//! 5. Validate/close fail on already-closed period

use chrono::NaiveDate;
use gl_rs::contracts::period_close_v1::{
    CloseStatusResponse, ClosePeriodRequest, ClosePeriodResponse,
    ValidateCloseRequest, ValidateCloseResponse, CloseStatus,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::{get_test_pool, setup_test_account, setup_test_period};

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM journal_lines WHERE tenant_id = $1")
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
    period_id: Uuid,
    account_code_debit: &str,
    account_code_credit: &str,
    amount_minor: i64,
) -> Uuid {
    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();

    // Insert journal entry
    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, period_id, entry_date, description, source_event_id, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(period_id)
    .bind(entry_date)
    .bind("Test balanced entry")
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("Failed to insert journal entry");

    // Insert debit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (
            id, journal_entry_id, tenant_id, account_ref, debit_minor, credit_minor, currency, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(tenant_id)
    .bind(account_code_debit)
    .bind(amount_minor)
    .bind(0)
    .bind("USD")
    .execute(pool)
    .await
    .expect("Failed to insert debit line");

    // Insert credit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (
            id, journal_entry_id, tenant_id, account_ref, debit_minor, credit_minor, currency, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(tenant_id)
    .bind(account_code_credit)
    .bind(0)
    .bind(amount_minor)
    .bind("USD")
    .execute(pool)
    .await
    .expect("Failed to insert credit line");

    entry_id
}

/// Helper to close a period directly via SQL (for testing idempotency and already-closed scenarios)
async fn close_period_directly(
    pool: &PgPool,
    period_id: Uuid,
    closed_by: &str,
    close_hash: &str,
) {
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
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_validate_close_success() {
    // Setup
    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-validate-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

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

    // ✅ BOUNDARY TEST: POST to validate-close endpoint
    let url = format!(
        "{}/api/gl/periods/{}/validate-close",
        gl_service_url, period_id
    );

    let client = reqwest::Client::new();
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
// TEST 2: Close Period (Success)
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_close_period_success() {
    // Setup
    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-close-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

    // Create open accounting period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Create balanced journal entry
    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    // ✅ BOUNDARY TEST: POST to close endpoint
    let url = format!("{}/api/gl/periods/{}/close", gl_service_url, period_id);

    let client = reqwest::Client::new();
    let request_body = ClosePeriodRequest {
        tenant_id: tenant_id.to_string(),
        closed_by: "admin".to_string(),
        close_reason: Some("Month-end close".to_string()),
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .expect("Failed to make HTTP request");

    // Assert: 200 OK
    assert_eq!(
        response.status(),
        200,
        "Expected 200 OK from close endpoint"
    );

    // Assert: Response is valid JSON
    let close_response: ClosePeriodResponse = response
        .json()
        .await
        .expect("Failed to parse JSON response");

    // Assert: Close succeeded
    assert_eq!(close_response.period_id, period_id);
    assert_eq!(close_response.tenant_id, tenant_id);
    assert!(close_response.success, "Close operation should succeed");
    assert!(
        close_response.close_status.is_some(),
        "Close status should be present"
    );

    let close_status = close_response.close_status.unwrap();
    match close_status {
        CloseStatus::Closed {
            closed_by,
            close_reason,
            close_hash,
            ..
        } => {
            assert_eq!(closed_by, "admin");
            assert_eq!(close_reason, Some("Month-end close".to_string()));
            assert!(!close_hash.is_empty(), "Close hash should be generated");

            // Verify period is actually closed in DB
            let db_period: (bool, Option<String>) = sqlx::query_as(
                "SELECT (closed_at IS NOT NULL), close_hash FROM accounting_periods WHERE id = $1",
            )
            .bind(period_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to fetch period");

            assert!(db_period.0, "Period should be closed in database");
            assert_eq!(
                db_period.1.as_deref(),
                Some(close_hash.as_str()),
                "Close hash should match DB"
            );
        }
        _ => panic!("Expected CloseStatus::Closed variant"),
    }

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 3: Idempotent Close (Repeated Calls)
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_close_period_idempotent() {
    // Setup
    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-idempotent-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

    // Create open accounting period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Create balanced journal entry
    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    let url = format!("{}/api/gl/periods/{}/close", gl_service_url, period_id);
    let client = reqwest::Client::new();
    let request_body = ClosePeriodRequest {
        tenant_id: tenant_id.to_string(),
        closed_by: "admin".to_string(),
        close_reason: Some("Month-end close".to_string()),
    };

    // ✅ FIRST CLOSE: Should succeed
    let response1 = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .expect("Failed to make first HTTP request");

    assert_eq!(response1.status(), 200, "First close should succeed");
    let close_response1: ClosePeriodResponse =
        response1.json().await.expect("Failed to parse response");
    assert!(close_response1.success);

    // Extract hash from first close
    let hash1 = match close_response1.close_status.unwrap() {
        CloseStatus::Closed { close_hash, .. } => close_hash,
        _ => panic!("Expected CloseStatus::Closed"),
    };

    // ✅ SECOND CLOSE: Should be idempotent (return same status)
    let response2 = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .expect("Failed to make second HTTP request");

    assert_eq!(
        response2.status(),
        200,
        "Second close should return 200 (idempotent)"
    );
    let close_response2: ClosePeriodResponse =
        response2.json().await.expect("Failed to parse response");
    assert!(close_response2.success, "Second close should indicate success (idempotent)");

    // Extract hash from second close
    let hash2 = match close_response2.close_status.unwrap() {
        CloseStatus::Closed { close_hash, .. } => close_hash,
        _ => panic!("Expected CloseStatus::Closed"),
    };

    // Assert: Idempotency - both hashes should be identical
    assert_eq!(
        hash1, hash2,
        "Idempotent close calls should return identical close_hash"
    );

    // Verify DB state hasn't changed
    let db_period: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM accounting_periods WHERE id = $1 AND closed_at IS NOT NULL")
            .bind(period_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query DB");

    assert_eq!(db_period.0, 1, "Should still have exactly one closed period");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 4: Close-Status Reflects Sealed Snapshot + Hash
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_close_status_reflects_snapshot() {
    // Setup
    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-status-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

    // Create and close accounting period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    // Close the period first
    let close_url = format!("{}/api/gl/periods/{}/close", gl_service_url, period_id);
    let client = reqwest::Client::new();
    let close_request = ClosePeriodRequest {
        tenant_id: tenant_id.to_string(),
        closed_by: "admin".to_string(),
        close_reason: Some("Test close".to_string()),
    };

    client
        .post(&close_url)
        .json(&close_request)
        .send()
        .await
        .expect("Failed to close period");

    // ✅ BOUNDARY TEST: GET close-status endpoint
    let status_url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        gl_service_url, period_id, tenant_id
    );

    let response = reqwest::get(&status_url)
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
// TEST 5: Validate-Close Fails on Already-Closed Period
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_validate_close_fails_on_closed_period() {
    // Setup
    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-validate-closed-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

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

    // ✅ BOUNDARY TEST: POST to validate-close on closed period
    let url = format!(
        "{}/api/gl/periods/{}/validate-close",
        gl_service_url, period_id
    );

    let client = reqwest::Client::new();
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
    assert!(!validate_response.can_close, "Should not allow closing an already-closed period");
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

// ============================================================
// TEST 6: Close Fails on Already-Closed Period
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_close_fails_on_closed_period() {
    // Setup
    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-close-closed-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    // Create Chart of Accounts
    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

    // Create and close accounting period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    // Close the period directly
    let test_hash = "test-hash-12345";
    close_period_directly(&pool, period_id, "admin", test_hash).await;

    // ✅ BOUNDARY TEST: Attempt to close an already-closed period
    // Per ChatGPT guardrail: idempotent operations should return existing status
    let url = format!("{}/api/gl/periods/{}/close", gl_service_url, period_id);

    let client = reqwest::Client::new();
    let request_body = ClosePeriodRequest {
        tenant_id: tenant_id.to_string(),
        closed_by: "different-user".to_string(),
        close_reason: Some("Attempting re-close".to_string()),
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .expect("Failed to make HTTP request");

    // Assert: 200 OK (idempotent - returns existing status)
    assert_eq!(
        response.status(),
        200,
        "Close on already-closed period should return 200 (idempotent)"
    );

    let close_response: ClosePeriodResponse =
        response.json().await.expect("Failed to parse response");

    // Assert: Returns existing close status (NOT a new close with different user)
    assert!(close_response.success);
    match close_response.close_status.unwrap() {
        CloseStatus::Closed {
            closed_by,
            close_hash,
            ..
        } => {
            // Should return ORIGINAL close info, not the new request
            assert_eq!(closed_by, "admin", "Should preserve original closed_by");
            assert_eq!(
                close_hash, test_hash,
                "Should preserve original close_hash"
            );
        }
        _ => panic!("Expected CloseStatus::Closed variant"),
    }

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 7: Performance Guard (< 1s per suite)
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Requires bd-3gr (HTTP Handlers) to be implemented"]
async fn test_boundary_http_period_close_performance_guard() {
    use std::time::Instant;

    // This test runs a subset of operations to ensure performance stays under 1s
    let start = Instant::now();

    let pool = get_test_pool().await;
    let tenant_id = "tenant-http-perf-001";
    let gl_service_url =
        std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string());

    cleanup_test_data(&pool, tenant_id).await;

    setup_test_account(&pool, tenant_id, "1100", "Cash", "ASSET", "DEBIT").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "REVENUE", "CREDIT").await;

    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    create_test_journal_entry(&pool, tenant_id, period_id, "1100", "4000", 100000).await;

    // Validate
    let validate_url = format!(
        "{}/api/gl/periods/{}/validate-close",
        gl_service_url, period_id
    );
    let client = reqwest::Client::new();
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
    reqwest::get(&status_url).await.expect("Status failed");

    cleanup_test_data(&pool, tenant_id).await;

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1000,
        "Period close workflow should complete in < 1s (actual: {}ms)",
        elapsed.as_millis()
    );
}
