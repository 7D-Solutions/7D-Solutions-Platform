//! Boundary E2E Test: HTTP Validate-Close Endpoint
//!
//! Tests the validate-close pre-flight endpoint:
//! - POST /api/gl/periods/{period_id}/validate-close
//!
//! ## IMPORTANT: Tests are #[ignore]d until bd-3gr is complete
//! These tests require HTTP handlers from bd-3gr (HTTP Handlers: Validate-Close, Close, Close-Status).
//! Once bd-3gr is merged, remove the #[ignore] attributes to enable these tests.
//!
//! ## Test Coverage
//! 1. Successful validation on open period with balanced entries
//! 2. Validation failure on already-closed period (PERIOD_ALREADY_CLOSED error)

use chrono::NaiveDate;
use gl_rs::contracts::period_close_v1::{ValidateCloseRequest, ValidateCloseResponse};
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
// TEST 2: Validate-Close Fails on Already-Closed Period
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
