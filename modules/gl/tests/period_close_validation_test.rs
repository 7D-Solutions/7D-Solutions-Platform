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

    let report = validate_period_can_close(&mut tx, tenant_id, period_id, false)
        .await
        .unwrap();

    // Should have blocking error: PERIOD_NOT_FOUND
    assert!(has_blocking_errors(&report));
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].code, "PERIOD_NOT_FOUND");
    assert_eq!(report.issues[0].severity, ValidationSeverity::Error);

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

    let report = validate_period_can_close(&mut tx, tenant_id, period_id, false)
        .await
        .unwrap();

    // Should have blocking error: PERIOD_ALREADY_CLOSED
    assert!(has_blocking_errors(&report));
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].code, "PERIOD_ALREADY_CLOSED");
    assert_eq!(report.issues[0].severity, ValidationSeverity::Error);

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
    .bind(period_start.and_hms_opt(12, 0, 0).unwrap().and_utc())
    .execute(&mut *tx)
    .await
    .unwrap();

    // Add lines that DON'T balance: 100 debit, 50 credit (net 50 out of balance)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES
            (gen_random_uuid(), $1, 1, '1000', 10000, 0),
            (gen_random_uuid(), $1, 2, '1000', 0, 5000)
        "#,
    )
    .bind(entry_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    let report = validate_period_can_close(&mut tx, tenant_id, period_id, false)
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
    assert_eq!(unbalanced_issue.severity, ValidationSeverity::Error);

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
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();
    let posted_at = period_start.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description
        )
        VALUES ($1, $2, 'test', $3, 'test.balanced', $4, 'USD', 'Balanced test entry')
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(source_event_id)
    .bind(posted_at)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Add balanced lines: 100 debit, 100 credit
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
        VALUES
            (gen_random_uuid(), $1, 1, '1000', 10000, 0),
            (gen_random_uuid(), $1, 2, '1000', 0, 10000)
        "#,
    )
    .bind(entry_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    let report = validate_period_can_close(&mut tx, tenant_id, period_id, false)
        .await
        .unwrap();

    // Should have NO blocking errors
    assert!(!has_blocking_errors(&report));
    assert!(
        report.issues.is_empty(),
        "Expected no issues, got: {:?}",
        report.issues
    );

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

    let report = validate_period_can_close(&mut tx, tenant_id, period_id, false)
        .await
        .unwrap();

    // Empty period should pass validation (no entries = no unbalanced entries)
    assert!(!has_blocking_errors(&report));
    assert!(report.issues.is_empty());

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: DLQ Validation - Disabled (Default)
// ============================================================

#[tokio::test]
async fn test_validate_dlq_disabled_with_pending_entries() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_dlq_disabled";
    let period_start = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();

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

    // Insert DLQ entries for posting-related subject
    let event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO failed_events (
            event_id, tenant_id, subject, envelope_json, error, retry_count, failed_at
        )
        VALUES ($1, $2, 'gl.events.posting.requested', '{}', 'Test error', 0, NOW())
        "#,
    )
    .bind(event_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Validate with DLQ validation DISABLED (default)
    let report = validate_period_can_close(&mut tx, tenant_id, period_id, false)
        .await
        .unwrap();

    // Should pass validation - DLQ entries should be IGNORED when disabled
    assert!(!has_blocking_errors(&report));
    assert!(
        report.issues.is_empty(),
        "Expected no issues when DLQ validation disabled, got: {:?}",
        report.issues
    );

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: DLQ Validation - Enabled with Pending Entries
// ============================================================

#[tokio::test]
async fn test_validate_dlq_enabled_with_pending_entries() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_dlq_enabled";
    let period_start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();

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

    // Insert DLQ entries for posting-related subject
    let event_id1 = Uuid::new_v4();
    let event_id2 = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO failed_events (
            event_id, tenant_id, subject, envelope_json, error, retry_count, failed_at
        )
        VALUES
            ($1, $2, 'gl.events.posting.requested', '{}', 'Test error 1', 0, NOW()),
            ($3, $2, 'gl.events.posting.requested', '{}', 'Test error 2', 1, NOW())
        "#,
    )
    .bind(event_id1)
    .bind(tenant_id)
    .bind(event_id2)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Validate with DLQ validation ENABLED
    let report = validate_period_can_close(&mut tx, tenant_id, period_id, true)
        .await
        .unwrap();

    // Should have blocking error: PENDING_DLQ_ENTRIES
    assert!(has_blocking_errors(&report));
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].code, "PENDING_DLQ_ENTRIES");
    assert_eq!(report.issues[0].severity, ValidationSeverity::Error);

    // Verify metadata contains correct count and subject filter
    let metadata = report.issues[0].metadata.as_ref().unwrap();
    assert_eq!(metadata["pending_dlq_count"], 2);
    assert_eq!(metadata["subject_filter"], "gl.events.posting.requested");

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: DLQ Validation - Enabled with No Pending Entries
// ============================================================

#[tokio::test]
async fn test_validate_dlq_enabled_no_pending_entries() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_dlq_enabled_empty";
    let period_start = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();

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

    // Validate with DLQ validation ENABLED but no DLQ entries
    let report = validate_period_can_close(&mut tx, tenant_id, period_id, true)
        .await
        .unwrap();

    // Should pass validation - no DLQ entries to block
    assert!(!has_blocking_errors(&report));
    assert!(report.issues.is_empty());

    tx.rollback().await.unwrap();
}

// ============================================================
// TEST: DLQ Validation - Tenant Scoping (Other Tenant's DLQ)
// ============================================================

#[tokio::test]
async fn test_validate_dlq_tenant_scoped() {
    let pool = get_test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant_id = "test_tenant_dlq_scoped";
    let other_tenant_id = "other_tenant_dlq_scoped";
    let period_start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();

    // Create open period for tenant
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

    // Insert DLQ entries for OTHER tenant (should NOT block our tenant)
    let event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO failed_events (
            event_id, tenant_id, subject, envelope_json, error, retry_count, failed_at
        )
        VALUES ($1, $2, 'gl.events.posting.requested', '{}', 'Test error', 0, NOW())
        "#,
    )
    .bind(event_id)
    .bind(other_tenant_id)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Validate with DLQ validation ENABLED
    let report = validate_period_can_close(&mut tx, tenant_id, period_id, true)
        .await
        .unwrap();

    // Should pass validation - other tenant's DLQ should NOT block us
    assert!(!has_blocking_errors(&report));
    assert!(
        report.issues.is_empty(),
        "Other tenant's DLQ should not block validation"
    );

    tx.rollback().await.unwrap();
}
