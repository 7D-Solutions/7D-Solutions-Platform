//! Boundary E2E Test: HTTP Close Period Endpoint
//!
//! Tests the close period endpoint:
//! - POST /api/gl/periods/{period_id}/close
//!
//! ## IMPORTANT: Tests are #[ignore]d until bd-3gr is complete
//! These tests require HTTP handlers from bd-3gr (HTTP Handlers: Validate-Close, Close, Close-Status).
//! Once bd-3gr is merged, remove the #[ignore] attributes to enable these tests.
//!
//! ## Test Coverage
//! 1. Successful close operation with snapshot + hash generation
//! 2. Idempotent close (repeated calls return identical hash)
//! 3. Close attempts on already-closed period (returns existing status)

use chrono::NaiveDate;
use gl_rs::contracts::period_close_v1::{CloseStatus, ClosePeriodRequest, ClosePeriodResponse};
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

/// Helper to close a period directly via SQL (for testing already-closed scenarios)
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
// TEST 1: Close Period (Success with Snapshot + Hash)
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
// TEST 2: Idempotent Close (Repeated Calls)
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
// TEST 3: Close Fails on Already-Closed Period (Idempotent)
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
