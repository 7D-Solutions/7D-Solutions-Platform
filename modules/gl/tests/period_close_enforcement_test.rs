//! Period Close Enforcement Integration Tests
//!
//! Tests for Phase 13 hard lock semantics:
//! - Posting blocked when period.closed_at is set
//! - Reversal blocked when original entry's period is closed
//!
//! Note: These tests are marked #[ignore] until bd-3rx (schema) and bd-1zp (close command) are complete.

use chrono::{NaiveDate, Utc};
use gl_rs::db::init_pool;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5438/gl_test".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

/// Helper to create a test period
async fn create_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Uuid {
    let period_id = Uuid::new_v4();

    sqlx::query(
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
    .await
    .expect("Failed to create test period");

    period_id
}

/// Helper to close a period (set closed_at)
/// TODO: Update this to use the actual close command from bd-1zp once available
async fn close_period(pool: &PgPool, period_id: Uuid, closed_by: &str) {
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = $1, closed_by = $2
        WHERE id = $3
        "#,
    )
    .bind(Utc::now())
    .bind(closed_by)
    .bind(period_id)
    .execute(pool)
    .await
    .expect("Failed to close period");
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
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

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM processed_events WHERE processor = 'test-processor'")
        .execute(pool)
        .await
        .ok();
}

// ============================================================
// TEST 1: Posting Blocked When Period Closed
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Waiting for bd-3rx (schema) - closed_at field not yet available"]
async fn test_posting_blocked_when_period_closed() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-close-001";

    // Setup: Create a period
    let period_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let period_id = create_test_period(&pool, tenant_id, period_start, period_end).await;

    // Close the period
    close_period(&pool, period_id, "test-admin").await;

    // TODO: Create test accounts
    // TODO: Attempt to post a journal entry to the closed period
    // TODO: Assert posting fails with PeriodError::PeriodClosed
    // TODO: Verify error message is stable and actionable

    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 2: Reversal Blocked When Original Period Closed
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Waiting for bd-3rx (schema) and bd-1zp (close command)"]
async fn test_reversal_blocked_when_original_period_closed() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-close-002";

    // Setup: Create two periods
    let period_a_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_a_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let period_a_id = create_test_period(&pool, tenant_id, period_a_start, period_a_end).await;

    let period_b_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_b_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let _period_b_id = create_test_period(&pool, tenant_id, period_b_start, period_b_end).await;

    // TODO: Create test accounts
    // TODO: Create a journal entry in period A
    // TODO: Close period A
    // TODO: Attempt to reverse the entry (reversal would go to period B)
    // TODO: Assert reversal fails with ReversalError::OriginalPeriodClosed
    // TODO: Verify error message includes original_entry_id, period_id, closed_at

    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 3: Reversal Blocked When Reversal Period Closed
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Waiting for bd-3rx (schema) and bd-1zp (close command)"]
async fn test_reversal_blocked_when_reversal_period_closed() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-close-003";

    // Setup: Create two periods
    let period_a_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_a_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let _period_a_id = create_test_period(&pool, tenant_id, period_a_start, period_a_end).await;

    let period_b_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_b_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let period_b_id = create_test_period(&pool, tenant_id, period_b_start, period_b_end).await;

    // TODO: Create test accounts
    // TODO: Create a journal entry in period A (leave open)
    // TODO: Close period B (the reversal period)
    // TODO: Attempt to reverse the entry (would fail because reversal period is closed)
    // TODO: Assert reversal fails with PeriodError::PeriodClosed
    // TODO: Verify existing enforcement still works

    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 4: Reversal Succeeds When Both Periods Open
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Waiting for bd-3rx (schema) - closed_at field not yet available"]
async fn test_reversal_succeeds_when_both_periods_open() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-close-004";

    // Setup: Create two periods (both open)
    let period_a_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_a_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let _period_a_id = create_test_period(&pool, tenant_id, period_a_start, period_a_end).await;

    let period_b_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_b_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let _period_b_id = create_test_period(&pool, tenant_id, period_b_start, period_b_end).await;

    // TODO: Create test accounts
    // TODO: Create a journal entry in period A
    // TODO: Reverse the entry (reversal in period B)
    // TODO: Assert reversal succeeds
    // TODO: Verify reversal entry was created
    // TODO: Verify reverses_entry_id links back to original

    cleanup_test_data(&pool, tenant_id).await;
}

// ============================================================
// TEST 5: closed_at Semantics Override is_closed Boolean
// ============================================================

#[tokio::test]
#[serial]
#[ignore = "Waiting for bd-3rx (schema) - closed_at field not yet available"]
async fn test_closed_at_semantics_override_is_closed_boolean() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-close-005";

    // Setup: Create a period with is_closed=false
    let period_start = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 2, 28).unwrap();
    let period_id = create_test_period(&pool, tenant_id, period_start, period_end).await;

    // Manually set closed_at (simulating close command) while leaving is_closed=false
    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET closed_at = $1, closed_by = $2, is_closed = false
        WHERE id = $3
        "#,
    )
    .bind(Utc::now())
    .bind("test-admin")
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to set closed_at");

    // TODO: Create test accounts
    // TODO: Attempt to post to the period
    // TODO: Assert posting fails (closed_at takes precedence over is_closed)
    // TODO: This proves Phase 13 semantics override Phase 10 boolean

    cleanup_test_data(&pool, tenant_id).await;
}
