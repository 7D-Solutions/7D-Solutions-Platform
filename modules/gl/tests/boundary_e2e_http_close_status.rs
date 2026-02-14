//! Boundary E2E Test: HTTP Close-Status Endpoint + Performance
//!
//! Tests the close-status query endpoint and performance guards:
//! - GET /api/gl/periods/{period_id}/close-status
//!
//! ## IMPORTANT: Tests are #[ignore]d until bd-3gr is complete
//! These tests require HTTP handlers from bd-3gr (HTTP Handlers: Validate-Close, Close, Close-Status).
//! Once bd-3gr is merged, remove the #[ignore] attributes to enable these tests.
//!
//! ## Test Coverage
//! 1. Close-status reflects sealed snapshot + hash after period close
//! 2. Performance guard ensures full workflow completes in < 1s

use chrono::NaiveDate;
use gl_rs::contracts::period_close_v1::{
    CloseStatus, ClosePeriodRequest, CloseStatusResponse, ValidateCloseRequest,
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

// ============================================================
// TEST 1: Close-Status Reflects Sealed Snapshot + Hash
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
// TEST 2: Performance Guard (< 1s per suite)
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
