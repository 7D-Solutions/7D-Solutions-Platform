//! Period Close Enforcement Integration Tests
//!
//! Tests for Phase 13 hard lock semantics:
//! - Posting blocked when period.closed_at is set
//! - Reversal blocked when original entry's period is closed
//!
//! ## Pool Configuration
//! These tests require DB_MAX_CONNECTIONS >= 5 due to transaction complexity
//! and cleanup cycles. Run with:
//! ```bash
//! DB_MAX_CONNECTIONS=5 cargo test --test period_close_enforcement_test
//! ```

mod common;

use chrono::{NaiveDate, Utc};
use common::get_test_pool;
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::{journal_service, reversal_service};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// TestCleanupGuard removed - Drop trait cannot safely block_on inside tokio runtime
// Instead, each test must call cleanup_test_data(&pool, &tenant_id).await explicitly

/// Helper to create a test period
async fn create_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Uuid {
    let period_id = Uuid::new_v4();

    eprintln!("[create_test_period] Attempting INSERT, pool: size={} idle={}", pool.size(), pool.num_idle());

    let result = sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .bind(false)
    .bind(Utc::now())
    .execute(pool)
    .await;

    match result {
        Ok(_) => {
            eprintln!("[create_test_period] INSERT succeeded, pool after: size={} idle={}", pool.size(), pool.num_idle());
            period_id
        }
        Err(e) => {
            eprintln!("[create_test_period] INSERT failed: {}, pool: size={} idle={}", e, pool.size(), pool.num_idle());
            panic!("Failed to create test period: {}", e);
        }
    }
}

/// Helper to close a period (set closed_at)
async fn close_period(pool: &PgPool, period_id: Uuid, closed_by: &str) {
    eprintln!("[close_period] Before UPDATE, pool: size={} idle={}", pool.size(), pool.num_idle());

    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = $1, closed_by = $2, close_hash = $3
        WHERE id = $4
        "#,
    )
    .bind(Utc::now())
    .bind(closed_by)
    .bind("test_hash_placeholder")
    .bind(period_id)
    .execute(pool)
    .await
    .expect("Failed to close period");

    eprintln!("[close_period] After UPDATE, pool: size={} idle={}", pool.size(), pool.num_idle());
}

/// Helper to create a test account
async fn create_test_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: AccountType,
    normal_balance: NormalBalance,
) -> Uuid {
    let id = Uuid::new_v4();

    eprintln!("[create_test_account] Before INSERT, pool: size={} idle={}", pool.size(), pool.num_idle());

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .bind(true)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test account");

    eprintln!("[create_test_account] After INSERT, pool: size={} idle={}", pool.size(), pool.num_idle());

    id
}

/// Helper to cleanup test data
///
/// Executes cleanup queries directly against pool (no transaction).
/// This minimizes connection hold time and prevents pool exhaustion.
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    eprintln!("[cleanup] START, pool: size={} idle={}", pool.size(), pool.num_idle());

    // Delete in reverse dependency order - execute directly without transaction
    // to minimize connection hold time
    eprintln!("[cleanup] Deleting journal_lines...");
    let _ = sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await;
    eprintln!("[cleanup] After journal_lines, pool: size={} idle={}", pool.size(), pool.num_idle());

    eprintln!("[cleanup] Deleting journal_entries...");
    let _ = sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;
    eprintln!("[cleanup] After journal_entries, pool: size={} idle={}", pool.size(), pool.num_idle());

    eprintln!("[cleanup] Deleting account_balances...");
    let _ = sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;
    eprintln!("[cleanup] After account_balances, pool: size={} idle={}", pool.size(), pool.num_idle());

    eprintln!("[cleanup] Deleting accounts...");
    let _ = sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;
    eprintln!("[cleanup] After accounts, pool: size={} idle={}", pool.size(), pool.num_idle());

    eprintln!("[cleanup] Deleting accounting_periods...");
    let _ = sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;
    eprintln!("[cleanup] After accounting_periods, pool: size={} idle={}", pool.size(), pool.num_idle());

    eprintln!("[cleanup] Deleting processed_events...");
    let _ = sqlx::query("DELETE FROM processed_events WHERE processor = 'test-processor'")
        .execute(pool)
        .await;
    eprintln!("[cleanup] After processed_events, pool: size={} idle={}", pool.size(), pool.num_idle());

    // Force PostgreSQL to reset connection state (clear temp tables, prepared statements, etc.)
    eprintln!("[cleanup] Running DISCARD ALL...");
    let _ = sqlx::query("DISCARD ALL")
        .execute(pool)
        .await;
    eprintln!("[cleanup] After DISCARD ALL, pool: size={} idle={}", pool.size(), pool.num_idle());

    eprintln!("[cleanup] END, pool: size={} idle={}", pool.size(), pool.num_idle());
}

// ============================================================
// TEST 1: Posting Blocked When Period Closed
// ============================================================

#[tokio::test]
#[serial]
async fn test_posting_blocked_when_period_closed() {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

    eprintln!("[TEST 1 START] Pool: size={} idle={}", pool.size(), pool.num_idle());

    // Setup: Create a period
    eprintln!("[TEST 1] Before create_test_period: size={} idle={}", pool.size(), pool.num_idle());
    let period_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let period_id = create_test_period(&pool, &tenant_id, period_start, period_end).await;
    eprintln!("[TEST 1] After create_test_period: size={} idle={}", pool.size(), pool.num_idle());

    // Create test accounts
    create_test_account(
        &pool,
        &tenant_id,
        "1200",
        "AR",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    create_test_account(
        &pool,
        &tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Close the period
    close_period(&pool, period_id, "test-admin").await;

    // Attempt to post a journal entry to the closed period
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_closed_001".to_string(),
        description: "Test posting to closed period".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    eprintln!("[TEST 1] Before journal_service: size={} idle={}", pool.size(), pool.num_idle());
    let result = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;
    eprintln!("[TEST 1] After journal_service: size={} idle={}", pool.size(), pool.num_idle());

    // Assert posting fails with PeriodClosed error
    assert!(result.is_err(), "Posting should fail when period is closed");

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    assert!(
        error_msg.contains("closed") || error_msg.contains("Accounting period is closed"),
        "Error should indicate period is closed: {}",
        error_msg
    );

    // Verify no journal entry was created (transaction rolled back)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(&tenant_id)
    .bind("inv_closed_001")
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal entries");

    assert_eq!(count, 0, "No journal entry should be created for failed posting");

    // Explicit cleanup to release DB connections
    cleanup_test_data(&pool, &tenant_id).await;

    // Brief delay to ensure connections return to pool
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}

// ============================================================
// TEST 2: Reversal Blocked When Original Period Closed
// ============================================================

#[tokio::test]
#[serial]
async fn test_reversal_blocked_when_original_period_closed() {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

    eprintln!("[TEST 2 START] Pool: size={} idle={}", pool.size(), pool.num_idle());

    // Setup: Create two periods using current year
    let period_a_start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let period_a_end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    let period_a_id = create_test_period(&pool, &tenant_id, period_a_start, period_a_end).await;

    let period_b_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let period_b_end = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
    let _period_b_id = create_test_period(&pool, &tenant_id, period_b_start, period_b_end).await;

    // Create test accounts
    create_test_account(
        &pool,
        &tenant_id,
        "1200",
        "AR",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    create_test_account(
        &pool,
        &tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Create a journal entry in period A
    let payload = GlPostingRequestV1 {
        posting_date: "2026-01-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_original_001".to_string(),
        description: "Original entry in period A".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    let original_entry_id = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await
    .expect("Failed to create original entry");

    // Close period A
    close_period(&pool, period_a_id, "test-admin").await;

    // Attempt to reverse the entry (reversal would go to period B which is open)
    let reversal_event_id = Uuid::new_v4();

    eprintln!("[TEST 2] Before reversal_service, pool: size={} idle={}", pool.size(), pool.num_idle());
    let result = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id,
        original_entry_id,
    )
    .await;
    eprintln!("[TEST 2] After reversal_service, pool: size={} idle={}", pool.size(), pool.num_idle());

    // Assert reversal fails with OriginalPeriodClosed error
    assert!(
        result.is_err(),
        "Reversal should fail when original period is closed"
    );

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    assert!(
        error_msg.contains("original period") || error_msg.contains("closed"),
        "Error should indicate original period is closed: {}",
        error_msg
    );

    assert!(
        error_msg.contains(&original_entry_id.to_string()),
        "Error should include original entry ID: {}",
        error_msg
    );

    // Verify no reversal entry was created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reverses_entry_id = $2",
    )
    .bind(&tenant_id)
    .bind(original_entry_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query reversal entries");

    assert_eq!(count, 0, "No reversal entry should be created");

    // Explicit cleanup to release DB connections
    cleanup_test_data(&pool, &tenant_id).await;

    // Brief delay to ensure connections return to pool
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}

// ============================================================
// TEST 3: Reversal Succeeds When Both Periods Open
// ============================================================

#[tokio::test]
#[serial]
async fn test_reversal_succeeds_when_both_periods_open() {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

    eprintln!("[TEST 3 START] Pool: size={} idle={}", pool.size(), pool.num_idle());

    // Setup: Create two periods (both open) using current year for reversal
    let period_a_start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let period_a_end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    let _period_a_id = create_test_period(&pool, &tenant_id, period_a_start, period_a_end).await;

    let period_b_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let period_b_end = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
    let _period_b_id = create_test_period(&pool, &tenant_id, period_b_start, period_b_end).await;

    // Create test accounts
    create_test_account(
        &pool,
        &tenant_id,
        "1200",
        "AR",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    create_test_account(
        &pool,
        &tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Create a journal entry in period A
    let payload = GlPostingRequestV1 {
        posting_date: "2026-01-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_open_001".to_string(),
        description: "Original entry in open period".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    let original_entry_id = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await
    .expect("Failed to create original entry");

    // Reverse the entry (both periods are open)
    let reversal_event_id = Uuid::new_v4();

    eprintln!("[TEST 3] Before reversal_service, pool: size={} idle={}", pool.size(), pool.num_idle());
    let result = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id,
        original_entry_id,
    )
    .await;
    eprintln!("[TEST 3] After reversal_service, pool: size={} idle={}", pool.size(), pool.num_idle());

    // Assert reversal succeeds
    if let Err(ref e) = result {
        panic!("Reversal should succeed when both periods are open. Error: {}", e);
    }

    let reversal_entry_id = result.unwrap();

    // Verify reversal entry was created
    let reversal_entry: Option<Uuid> = sqlx::query_scalar(
        "SELECT reverses_entry_id FROM journal_entries WHERE id = $1",
    )
    .bind(reversal_entry_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query reversal entry");

    assert_eq!(
        reversal_entry,
        Some(original_entry_id),
        "Reversal entry should link back to original"
    );

    // Explicit cleanup to release DB connections
    cleanup_test_data(&pool, &tenant_id).await;

    // Brief delay to ensure connections return to pool
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}

// ============================================================
// TEST 4: closed_at Semantics Override is_closed Boolean
// ============================================================

#[tokio::test]
#[serial]
async fn test_closed_at_semantics_override_is_closed_boolean() {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

    eprintln!("[TEST 4 START] Pool: size={} idle={}", pool.size(), pool.num_idle());

    // Setup: Create a period with is_closed=false
    let period_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let period_id = create_test_period(&pool, &tenant_id, period_start, period_end).await;

    // Create test accounts
    create_test_account(
        &pool,
        &tenant_id,
        "1200",
        "AR",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    create_test_account(
        &pool,
        &tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Manually set closed_at while leaving is_closed=false
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = $1, closed_by = $2, close_hash = $3, is_closed = false
        WHERE id = $4
        "#,
    )
    .bind(Utc::now())
    .bind("test-admin")
    .bind("test_hash")
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to set closed_at");

    // Attempt to post to the period
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_semantics_001".to_string(),
        description: "Test closed_at semantics".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    let result = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    // Assert posting fails (closed_at takes precedence over is_closed)
    assert!(
        result.is_err(),
        "Posting should fail - closed_at takes precedence over is_closed=false"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("closed"),
        "Error should indicate period is closed: {}",
        error_msg
    );

    // Explicit cleanup to release DB connections
    cleanup_test_data(&pool, &tenant_id).await;

    // Brief delay to ensure connections return to pool
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}
