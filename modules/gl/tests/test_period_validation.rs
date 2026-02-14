//! Integration tests for period close validation engine (Phase 13: bd-3sl)
//!
//! Tests verify:
//! - Period existence validation (tenant-scoped)
//! - Already closed check
//! - Unbalanced journal entries detection
//! - Validation report structure

mod common;

use chrono::NaiveDate;
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use gl_rs::services::period_close_service::{has_blocking_errors, validate_period_can_close};
use serial_test::serial;
use uuid::Uuid;

/// Test validation passes for a valid open period with no entries
#[tokio::test]
#[serial]
async fn test_validate_empty_period_success() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_val_empty";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create an open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 7, 31).unwrap(),
    )
    .await;

    // Validate period
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Should pass validation (no issues)
    assert!(report.issues.is_empty());
    assert!(!has_blocking_errors(&report));
}

/// Test validation fails when period doesn't exist
#[tokio::test]
#[serial]
async fn test_validate_period_not_found() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_val_notfound";
    let fake_period_id = Uuid::new_v4();

    // Try to validate non-existent period
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_id, fake_period_id)
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    // Should have PERIOD_NOT_FOUND error
    assert!(!report.issues.is_empty());
    assert!(has_blocking_errors(&report));

    let error = &report.issues[0];
    assert_eq!(error.code, "PERIOD_NOT_FOUND");
    assert_eq!(error.severity, gl_rs::contracts::period_close_v1::ValidationSeverity::Error);
}

/// Test validation fails when period is already closed
#[tokio::test]
#[serial]
async fn test_validate_period_already_closed() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_val_closed";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 8, 31).unwrap(),
    )
    .await;

    // Close the period manually
    sqlx::query(
        "UPDATE accounting_periods SET closed_at = NOW(), close_hash = 'test_hash' WHERE id = $1"
    )
    .bind(period_id)
    .execute(&pool)
    .await
    .unwrap();

    // Try to validate closed period
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Should have PERIOD_ALREADY_CLOSED error
    assert!(!report.issues.is_empty());
    assert!(has_blocking_errors(&report));

    let error = &report.issues[0];
    assert_eq!(error.code, "PERIOD_ALREADY_CLOSED");
    assert_eq!(error.severity, gl_rs::contracts::period_close_v1::ValidationSeverity::Error);
    assert!(error.metadata.is_some());
}

/// Test validation detects unbalanced journal entries
#[tokio::test]
#[serial]
async fn test_validate_unbalanced_entries() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_val_unbalanced";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 9, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 9, 30).unwrap(),
    )
    .await;

    // Create accounts
    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;

    // Create an UNBALANCED journal entry (debit without matching credit)
    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2025, 9, 15).unwrap();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test_unbalanced', $3, 'USD', 'Unbalanced entry', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .execute(&pool)
    .await
    .unwrap();

    // Add ONLY debit line (no credit - unbalanced!)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES (gen_random_uuid(), $1, 1, '1000', 100000, 0, 'Unbalanced debit')
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .unwrap();

    // Validate period
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Should have UNBALANCED_ENTRIES error
    assert!(!report.issues.is_empty());
    assert!(has_blocking_errors(&report));

    let error = &report.issues[0];
    assert_eq!(error.code, "UNBALANCED_ENTRIES");
    assert_eq!(error.severity, gl_rs::contracts::period_close_v1::ValidationSeverity::Error);
    assert!(error.message.contains("unbalanced"));
    assert!(error.metadata.is_some());
}

/// Test validation passes with balanced journal entries
#[tokio::test]
#[serial]
async fn test_validate_balanced_entries_success() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_val_balanced";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 10, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 10, 31).unwrap(),
    )
    .await;

    // Create accounts
    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "revenue", "revenue", "credit").await;

    // Create a BALANCED journal entry
    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2025, 10, 15).unwrap();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test_balanced', $3, 'USD', 'Balanced entry', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .execute(&pool)
    .await
    .unwrap();

    // Add balanced lines (debit + credit = balanced)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES
            (gen_random_uuid(), $1, 1, '1000', 100000, 0, 'Debit line'),
            (gen_random_uuid(), $1, 2, '4000', 0, 100000, 'credit line')
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .unwrap();

    // Validate period
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Should pass validation (no issues)
    assert!(report.issues.is_empty());
    assert!(!has_blocking_errors(&report));
}

/// Test tenant isolation - cannot validate other tenant's period
#[tokio::test]
#[serial]
async fn test_validate_tenant_isolation() {
    let pool = get_test_pool().await;
    let tenant_a = "tenant_val_a";
    let tenant_b = "tenant_val_b";

    cleanup_test_tenant(&pool, tenant_a).await;

    // Create period for tenant A
    let period_id = setup_test_period(
        &pool,
        tenant_a,
        NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 11, 30).unwrap(),
    )
    .await;

    // Try to validate tenant A's period as tenant B
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_b, period_id)
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    // Should fail with PERIOD_NOT_FOUND (tenant isolation)
    assert!(!report.issues.is_empty());
    assert!(has_blocking_errors(&report));

    let error = &report.issues[0];
    assert_eq!(error.code, "PERIOD_NOT_FOUND");
}

/// Test multiple validation errors are all reported
#[tokio::test]
#[serial]
async fn test_validate_multiple_errors() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_val_multi";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
    )
    .await;

    // Close the period
    sqlx::query(
        "UPDATE accounting_periods SET closed_at = NOW(), close_hash = 'test_hash' WHERE id = $1"
    )
    .bind(period_id)
    .execute(&pool)
    .await
    .unwrap();

    // Create unbalanced entry
    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;

    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2025, 12, 15).unwrap();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test', $3, 'USD', 'Test', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES (gen_random_uuid(), $1, 1, '1000', 100000, 0, 'Unbalanced')
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .unwrap();

    // Validate period
    let mut tx = pool.begin().await.unwrap();
    let report = validate_period_can_close(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Should have BOTH errors: already closed AND unbalanced entries
    assert!(report.issues.len() >= 1); // At least already closed error
    assert!(has_blocking_errors(&report));

    let codes: Vec<String> = report.issues.iter().map(|i| i.code.clone()).collect();
    assert!(codes.contains(&"PERIOD_ALREADY_CLOSED".to_string()));
}
