//! Pre-Close Validation Engine Integration Tests
//!
//! Phase 13 - bd-3sl: Tests for period close validation logic
//! Validates mandatory checks: period exists, not closed, balanced entries

mod common;

use chrono::{NaiveDate, Utc};
use common::get_test_pool;
use gl_rs::contracts::period_close_v1::ValidationSeverity;
use gl_rs::services::period_close_service::{has_blocking_errors, validate_period_can_close};
use serial_test::serial;
use uuid::Uuid;

// ============================================================
// TEST: Period Not Found
// ============================================================

#[tokio::test]
#[serial]
async fn test_validate_period_not_found() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_not_found";
    let period_id = Uuid::new_v4(); // Non-existent period

    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();

    // Should have blocking error: PERIOD_NOT_FOUND
    assert!(has_blocking_errors(&report));
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].code, "PERIOD_NOT_FOUND");
    assert_eq!(
        report.issues[0].severity,
        ValidationSeverity::Error
    );

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: Period Already Closed
// ============================================================

#[tokio::test]
async fn test_validate_period_already_closed() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_already_closed";
    let period_start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

    // Create a period that is already closed
    let period_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO accounting_periods (
            tenant_id, period_start, period_end, closed_at, close_hash
        )
        VALUES ($1, $2, $3, $4, 'dummy_hash')
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .bind(Utc::now())
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();

    // Should have blocking error: PERIOD_ALREADY_CLOSED
    assert!(has_blocking_errors(&report));
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].code, "PERIOD_ALREADY_CLOSED");
    assert_eq!(
        report.issues[0].severity,
        ValidationSeverity::Error
    );

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: Unbalanced Journal Entries
// ============================================================

#[tokio::test]
async fn test_validate_unbalanced_entries() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_unbalanced";
    let period_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

    // Create open period
    let period_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Create an account for testing
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance)
        VALUES ($1, $2, '1000', 'Test Account', 'asset', 'debit')
        "#,
    )
    .bind(account_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Create an UNBALANCED journal entry (deliberate test case)
    // This should never happen in production, but validation must catch it
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id, source_subject,
            posted_at, currency, description
        )
        VALUES ($1, $2, 'test', $3, 'test.validation', $4, 'USD', 'Unbalanced test entry')
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(source_event_id)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();

    // Add lines that DON'T balance: 100 debit, 50 credit (net 50 out of balance)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (journal_entry_id, account_id, debit_minor, credit_minor)
        VALUES ($1, $2, 10000, 0), ($1, $2, 0, 5000)
        "#,
    )
    .bind(entry_id)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();

    // Should have blocking error: UNBALANCED_ENTRIES
    assert!(has_blocking_errors(&report));
    assert!(report.issues.iter().any(|i| i.code == "UNBALANCED_ENTRIES"));

    let unbalanced_issue = report
        .issues
        .iter()
        .find(|i| i.code == "UNBALANCED_ENTRIES")
        .unwrap();
    assert_eq!(
        unbalanced_issue.severity,
        ValidationSeverity::Error
    );

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: Valid Period (All Checks Pass)
// ============================================================

#[tokio::test]
async fn test_validate_valid_period() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_valid";
    let period_start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();

    // Create open period
    let period_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Create an account for testing
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance)
        VALUES ($1, $2, '1000', 'Test Account', 'asset', 'debit')
        "#,
    )
    .bind(account_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Create a BALANCED journal entry
    let entry_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO journal_entries (
            tenant_id, entry_date, posted_at, currency, description
        )
        VALUES ($1, $2, $3, 'USD', 'Balanced test entry')
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(Utc::now())
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Add balanced lines: 100 debit, 100 credit
    sqlx::query(
        r#"
        INSERT INTO journal_lines (journal_entry_id, account_id, debit_minor, credit_minor)
        VALUES ($1, $2, 10000, 0), ($1, $2, 0, 10000)
        "#,
    )
    .bind(entry_id)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();

    // Should have NO blocking errors
    assert!(!has_blocking_errors(&report));
    assert!(report.issues.is_empty(), "Expected no issues, got: {:?}", report.issues);

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: Empty Period (No Entries)
// ============================================================

#[tokio::test]
async fn test_validate_empty_period() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_empty";
    let period_start = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();

    // Create open period with no journal entries
    let period_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();

    // Empty period should pass validation (no entries = no unbalanced entries)
    assert!(!has_blocking_errors(&report));
    assert!(report.issues.is_empty());

    tx.rollback().await.unwrap();
}
