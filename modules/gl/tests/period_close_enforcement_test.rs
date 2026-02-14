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
use sqlx::{Connection, PgPool, Row};
use std::time::{Duration, Instant};
use uuid::Uuid;

// TestCleanupGuard removed - Drop trait cannot safely block_on inside tokio runtime
// Instead, each test must call cleanup_test_data(&pool, &tenant_id).await explicitly

// NOTE: Cross-binary advisory lock is acquired once in get_test_pool() (common/mod.rs)
// and held for the lifetime of the test binary. This serializes test binaries.

/// ChatGPT diagnostic: Dump Postgres activity and locks to identify smoking guns
///
/// Smoking guns to look for:
/// 1. state='idle in transaction' → tx leak (real problem)
/// 2. wait_event_type='Lock' → lock contention blocking INSERT
/// 3. Long state_change age with locks → truly stuck session
///
/// Note: state='idle' + wait_event='ClientRead' is NORMAL for pooled connections
async fn dump_pg_activity() {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string());

    match sqlx::PgConnection::connect(&database_url).await {
        Ok(mut conn) => {
            // Expanded activity with transaction and state change timestamps
            let activity_result = sqlx::query(
                r#"
                SELECT pid, state, wait_event_type, wait_event, xact_start, state_change, query
                FROM pg_stat_activity
                WHERE datname = current_database()
                ORDER BY state, pid
                "#
            )
            .fetch_all(&mut conn)
            .await;

            match activity_result {
                Ok(rows) => {
                    eprintln!("[DIAGNOSTIC] pg_stat_activity dump ({} rows):", rows.len());
                    for row in rows {
                        let pid: i32 = row.try_get("pid").unwrap_or(0);
                        let state: Option<String> = row.try_get("state").ok();
                        let wait_event_type: Option<String> = row.try_get("wait_event_type").ok();
                        let wait_event: Option<String> = row.try_get("wait_event").ok();
                        let xact_start: Option<chrono::DateTime<chrono::Utc>> = row.try_get("xact_start").ok();
                        let state_change: Option<chrono::DateTime<chrono::Utc>> = row.try_get("state_change").ok();
                        let query: Option<String> = row.try_get("query").ok();

                        eprintln!("[DIAGNOSTIC]   pid={}, state={:?}, wait_type={:?}, wait_event={:?}, xact_start={:?}, state_change={:?}, query={:?}",
                            pid, state, wait_event_type, wait_event, xact_start, state_change, query
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[DIAGNOSTIC] Failed to fetch pg_stat_activity: {}", e);
                }
            }

            // Lock analysis for period-related tables
            let locks_result = sqlx::query(
                r#"
                SELECT l.pid, l.locktype, l.mode, l.granted, c.relname
                FROM pg_locks l
                LEFT JOIN pg_class c ON l.relation = c.oid
                WHERE c.relname IN ('accounting_periods', 'accounts', 'journal_entries', 'journal_lines')
                "#
            )
            .fetch_all(&mut conn)
            .await;

            match locks_result {
                Ok(rows) => {
                    eprintln!("[DIAGNOSTIC] pg_locks dump ({} rows):", rows.len());
                    for row in rows {
                        let pid: i32 = row.try_get("pid").unwrap_or(0);
                        let locktype: Option<String> = row.try_get("locktype").ok();
                        let mode: Option<String> = row.try_get("mode").ok();
                        let granted: Option<bool> = row.try_get("granted").ok();
                        let relname: Option<String> = row.try_get("relname").ok();

                        eprintln!("[DIAGNOSTIC]   pid={}, locktype={:?}, mode={:?}, granted={:?}, table={:?}",
                            pid, locktype, mode, granted, relname
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[DIAGNOSTIC] Failed to fetch pg_locks: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("[DIAGNOSTIC] Failed to connect for diagnostics: {}", e);
        }
    }
}

/// Assert that all connections have been returned to the pool (bounded wait).
///
/// SQLx can keep connections "checked out" for async scheduler bookkeeping until
/// the next yield point. This helper waits up to 250ms for connections to drain
/// back to idle state.
///
/// This is NOT a sleep hack - it's a deterministic drain check with a tight bound.
async fn assert_pool_drained(pool: &PgPool) {
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        let size = pool.size();
        let idle = pool.num_idle() as u32;
        let checked_out = size.saturating_sub(idle);

        if checked_out == 0 {
            return; // Success!
        }

        if Instant::now() >= deadline {
            panic!(
                "Connection still checked out after test! checked_out={}, idle={}, size={}",
                checked_out, idle, size
            );
        }

        tokio::task::yield_now().await;
    }
}

/// Helper to create a test period
async fn create_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Uuid {
    let period_id = Uuid::new_v4();

    // ChatGPT diagnostic: Log pool state RIGHT BEFORE the INSERT
    eprintln!("[DIAGNOSTIC] before create_test_period INSERT: size={}, idle={}, checked_out={}, tenant={}",
        pool.size(),
        pool.num_idle(),
        pool.size().saturating_sub(pool.num_idle() as u32),
        tenant_id
    );

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

    // ChatGPT diagnostic: Dump pg_stat_activity on failure
    if result.is_err() {
        eprintln!("[DIAGNOSTIC] create_test_period INSERT failed, dumping pg_stat_activity");
        dump_pg_activity().await;
    }

    let result = result.expect("Failed to create test period");

    // Explicitly drop the query result to force connection release
    drop(result);

    period_id
}

/// Helper to close a period (set closed_at)
async fn close_period(pool: &PgPool, period_id: Uuid, closed_by: &str) {
    let result = sqlx::query(
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

    // Explicitly drop the query result to force connection release
    drop(result);
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

    let result = sqlx::query(
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

    // Explicitly drop the query result to force connection release
    drop(result);

    id
}

/// Helper to cleanup test data
///
/// ChatGPT CORRECTED Fix: Wrap cleanup in transaction to ensure atomic cleanup
/// and prevent idle-in-transaction states that cause connection corruption.
///
/// **Error propagation:** Propagate errors instead of swallowing them to catch cleanup failures early.
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;

    // Delete in reverse FK dependency order (children → parents)

    // 1. Journal lines (FK to journal_entries)
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)"
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    // 2. Journal entries
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    // 3. Account balances
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    // 4. Period summary snapshots
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    // 5. Accounts
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    // 6. Accounting periods
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================
// TEST 1: Posting Blocked When Period Closed
// ============================================================

#[tokio::test]
#[serial]
async fn test_posting_blocked_when_period_closed() -> Result<(), sqlx::Error> {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

    // Setup: Create a period
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

    let result = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        &tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

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
    cleanup_test_data(&pool, &tenant_id).await?;

    // ChatGPT Phase 13 requirement: Verify all connections returned to pool
    // Bounded wait for connections to drain (handles async scheduler bookkeeping)
    assert_pool_drained(&pool).await;

    Ok(())
}

// ============================================================
// TEST 2: Reversal Blocked When Original Period Closed
// ============================================================

#[tokio::test]
#[serial]
async fn test_reversal_blocked_when_original_period_closed() -> Result<(), sqlx::Error> {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

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

    let result = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id,
        original_entry_id,
    )
    .await;

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
    cleanup_test_data(&pool, &tenant_id).await?;

    // ChatGPT Phase 13 requirement: Verify all connections returned to pool
    // Bounded wait for connections to drain (handles async scheduler bookkeeping)
    assert_pool_drained(&pool).await;

    Ok(())
}

// ============================================================
// TEST 3: Reversal Succeeds When Both Periods Open
// ============================================================

#[tokio::test]
#[serial]
async fn test_reversal_succeeds_when_both_periods_open() -> Result<(), sqlx::Error> {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

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

    let result = reversal_service::create_reversal_entry(
        &pool,
        reversal_event_id,
        original_entry_id,
    )
    .await;

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
    cleanup_test_data(&pool, &tenant_id).await?;

    // ChatGPT Phase 13 requirement: Verify all connections returned to pool
    // Bounded wait for connections to drain (handles async scheduler bookkeeping)
    assert_pool_drained(&pool).await;

    Ok(())
}

// ============================================================
// TEST 4: closed_at Semantics Override is_closed Boolean
// ============================================================

#[tokio::test]
#[serial]
async fn test_closed_at_semantics_override_is_closed_boolean() -> Result<(), sqlx::Error> {
    let pool = get_test_pool().await;
    let tenant_id = format!("tenant-close-{}", Uuid::new_v4());

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
    cleanup_test_data(&pool, &tenant_id).await?;

    // ChatGPT Phase 13 requirement: Verify all connections returned to pool
    // Bounded wait for connections to drain (handles async scheduler bookkeeping)
    assert_pool_drained(&pool).await;

    Ok(())
}
