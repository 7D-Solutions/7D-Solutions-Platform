//! Integration tests for period close snapshot sealing (Phase 13: bd-1hi)
//!
//! Tests verify:
//! - Deterministic hash computation
//! - Snapshot persistence in transaction
//! - Hash reproducibility
//! - Currency snapshot accuracy

mod common;

use chrono::NaiveDate;
use common::{get_test_pool, setup_test_account, setup_test_period};
use gl_rs::services::period_close_service::{
    compute_close_hash, create_close_snapshot, verify_close_hash, PeriodCloseError,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Test that compute_close_hash produces deterministic results
#[tokio::test]
#[serial]
async fn test_close_hash_deterministic() {
    let tenant_id = "test_tenant";
    let period_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let hash1 = compute_close_hash(tenant_id, period_id, 10, 100000, 100000, 5);
    let hash2 = compute_close_hash(tenant_id, period_id, 10, 100000, 100000, 5);

    // Same inputs must produce same hash (deterministic)
    assert_eq!(hash1, hash2);

    // Hash must be SHA-256 (64 hex characters)
    assert_eq!(hash1.len(), 64);
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));
}

/// Test that different inputs produce different hashes
#[tokio::test]
#[serial]
async fn test_close_hash_different_inputs() {
    let tenant_id = "test_tenant";
    let period_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let hash1 = compute_close_hash(tenant_id, period_id, 10, 100000, 100000, 5);
    let hash2 = compute_close_hash(tenant_id, period_id, 11, 100000, 100000, 5); // Different journal count
    let hash3 = compute_close_hash(tenant_id, period_id, 10, 200000, 100000, 5); // Different debits
    let hash4 = compute_close_hash(tenant_id, period_id, 10, 100000, 200000, 5); // Different credits
    let hash5 = compute_close_hash(tenant_id, period_id, 10, 100000, 100000, 6); // Different balance count

    // All different inputs must produce different hashes
    assert_ne!(hash1, hash2);
    assert_ne!(hash1, hash3);
    assert_ne!(hash1, hash4);
    assert_ne!(hash1, hash5);
}

/// Test creating a close snapshot with no journal entries (empty period)
#[tokio::test]
#[serial]
async fn test_create_close_snapshot_empty_period() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_snap_empty";

    // Cleanup any existing test data
    cleanup_test_tenant(&pool, tenant_id).await;

    // Create a period (2025-01-01 to 2025-01-31 to avoid conflicts with other tests)
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
    )
    .await;

    // Create close snapshot in a transaction
    let mut tx = pool.begin().await.unwrap();
    let snapshot = create_close_snapshot(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Verify snapshot properties
    assert_eq!(snapshot.tenant_id, tenant_id);
    assert_eq!(snapshot.period_id, period_id);
    assert_eq!(snapshot.total_journal_count, 0);
    assert_eq!(snapshot.total_debits_minor, 0);
    assert_eq!(snapshot.total_credits_minor, 0);
    assert_eq!(snapshot.balance_row_count, 0);
    assert_eq!(snapshot.currency_snapshots.len(), 0); // No currencies
    assert_eq!(snapshot.close_hash.len(), 64); // SHA-256 hex

    // Verify hash is reproducible
    let expected_hash = compute_close_hash(tenant_id, period_id, 0, 0, 0, 0);
    assert_eq!(snapshot.close_hash, expected_hash);
}

/// Test creating a close snapshot with journal entries
#[tokio::test]
#[serial]
async fn test_create_close_snapshot_with_entries() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_snap_entries";

    // Cleanup any existing test data
    cleanup_test_tenant(&pool, tenant_id).await;

    // Setup: Create period and accounts
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
    )
    .await;

    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "revenue", "revenue", "credit").await;

    // Create journal entries (2 entries, 4 lines, in USD and EUR)
    create_test_journal_entry(
        &pool,
        tenant_id,
        period_id,
        "1000", // account code
        "4000", // account code
        "USD",
        100000, // $1000.00
        NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
    )
    .await;

    create_test_journal_entry(
        &pool,
        tenant_id,
        period_id,
        "1000", // account code
        "4000", // account code
        "EUR",
        50000, // €500.00
        NaiveDate::from_ymd_opt(2024, 2, 20).unwrap(),
    )
    .await;

    // Create close snapshot
    let mut tx = pool.begin().await.unwrap();
    let snapshot = create_close_snapshot(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Verify snapshot aggregates
    assert_eq!(snapshot.total_journal_count, 2);
    assert_eq!(snapshot.total_debits_minor, 150000); // 100000 + 50000
    assert_eq!(snapshot.total_credits_minor, 150000);
    assert_eq!(snapshot.currency_snapshots.len(), 2); // USD and EUR

    // Verify currency snapshots
    let usd_snapshot = snapshot
        .currency_snapshots
        .iter()
        .find(|s| s.currency == "USD")
        .expect("USD snapshot should exist");
    assert_eq!(usd_snapshot.journal_count, 1);
    assert_eq!(usd_snapshot.line_count, 2);
    assert_eq!(usd_snapshot.total_debits_minor, 100000);
    assert_eq!(usd_snapshot.total_credits_minor, 100000);

    let eur_snapshot = snapshot
        .currency_snapshots
        .iter()
        .find(|s| s.currency == "EUR")
        .expect("EUR snapshot should exist");
    assert_eq!(eur_snapshot.journal_count, 1);
    assert_eq!(eur_snapshot.line_count, 2);
    assert_eq!(eur_snapshot.total_debits_minor, 50000);
    assert_eq!(eur_snapshot.total_credits_minor, 50000);

    // Verify hash is reproducible (balance_count will be 0 since we didn't create balances)
    let expected_hash = compute_close_hash(
        tenant_id,
        period_id,
        2,
        150000,
        150000,
        0, // No account_balances created in test
    );
    assert_eq!(snapshot.close_hash, expected_hash);
}

/// Test that snapshots are persisted to database
#[tokio::test]
#[serial]
async fn test_snapshot_persistence() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_snap_persist";

    // Cleanup any existing test data
    cleanup_test_tenant(&pool, tenant_id).await;

    // Setup period and entry
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
    )
    .await;

    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "revenue", "revenue", "credit").await;

    create_test_journal_entry(
        &pool,
        tenant_id,
        period_id,
        "1000",
        "4000",
        "USD",
        75000,
        NaiveDate::from_ymd_opt(2024, 3, 15).unwrap(),
    )
    .await;

    // Create snapshot
    let mut tx = pool.begin().await.unwrap();
    let _snapshot = create_close_snapshot(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Verify snapshot was persisted to database
    let persisted_snapshot: Option<(String, i32, i32, i64, i64)> = sqlx::query_as(
        r#"
        SELECT currency, journal_count, line_count, total_debits_minor, total_credits_minor
        FROM period_summary_snapshots
        WHERE tenant_id = $1 AND period_id = $2 AND currency = $3
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind("USD")
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert!(persisted_snapshot.is_some());
    let (currency, journal_count, line_count, debits, credits) = persisted_snapshot.unwrap();
    assert_eq!(currency, "USD");
    assert_eq!(journal_count, 1);
    assert_eq!(line_count, 2);
    assert_eq!(debits, 75000);
    assert_eq!(credits, 75000);
}

/// Test hash verification with matching hash
#[tokio::test]
#[serial]
async fn test_verify_close_hash_success() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_snap_verify_ok";

    // Cleanup any existing test data
    cleanup_test_tenant(&pool, tenant_id).await;

    // Setup period and entry
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 4, 30).unwrap(),
    )
    .await;

    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "revenue", "revenue", "credit").await;

    create_test_journal_entry(
        &pool,
        tenant_id,
        period_id,
        "1000",
        "4000",
        "USD",
        25000,
        NaiveDate::from_ymd_opt(2024, 4, 15).unwrap(),
    )
    .await;

    // Create snapshot and get hash
    let mut tx = pool.begin().await.unwrap();
    let snapshot = create_close_snapshot(&mut tx, tenant_id, period_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Verify hash matches
    let result = verify_close_hash(&pool, tenant_id, period_id, &snapshot.close_hash).await;
    assert!(result.is_ok());
}

/// Test hash verification with mismatched hash
#[tokio::test]
#[serial]
async fn test_verify_close_hash_failure() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_snap_verify_fail";

    // Cleanup any existing test data
    cleanup_test_tenant(&pool, tenant_id).await;

    // Setup period and entry (2025-05-01 to avoid conflicts)
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 5, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 5, 31).unwrap(),
    )
    .await;

    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "revenue", "revenue", "credit").await;

    create_test_journal_entry(
        &pool,
        tenant_id,
        period_id,
        "1000",
        "4000",
        "USD",
        30000,
        NaiveDate::from_ymd_opt(2025, 5, 15).unwrap(),
    )
    .await;

    // Try to verify with wrong hash
    let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let result = verify_close_hash(&pool, tenant_id, period_id, wrong_hash).await;

    assert!(result.is_err());
    match result {
        Err(PeriodCloseError::HashMismatch { computed, expected }) => {
            assert_eq!(expected, wrong_hash);
            assert_ne!(computed, expected);
        }
        _ => panic!("Expected HashMismatch error"),
    }
}

/// Test snapshot idempotency (calling create_close_snapshot twice)
#[tokio::test]
#[serial]
async fn test_snapshot_idempotency() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_snap_idempotent";

    // Cleanup any existing test data
    cleanup_test_tenant(&pool, tenant_id).await;

    // Setup period and entry (2025-06-01 to avoid conflicts)
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(),
    )
    .await;

    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "revenue", "revenue", "credit").await;

    create_test_journal_entry(
        &pool,
        tenant_id,
        period_id,
        "1000",
        "4000",
        "USD",
        40000,
        NaiveDate::from_ymd_opt(2025, 6, 15).unwrap(),
    )
    .await;

    // Create snapshot first time
    let mut tx1 = pool.begin().await.unwrap();
    let snapshot1 = create_close_snapshot(&mut tx1, tenant_id, period_id)
        .await
        .unwrap();
    tx1.commit().await.unwrap();

    // Create snapshot second time (should be idempotent)
    let mut tx2 = pool.begin().await.unwrap();
    let snapshot2 = create_close_snapshot(&mut tx2, tenant_id, period_id)
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // Both snapshots should have the same hash
    assert_eq!(snapshot1.close_hash, snapshot2.close_hash);

    // Verify only one snapshot row exists in DB (ON CONFLICT DO UPDATE)
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM period_summary_snapshots
        WHERE tenant_id = $1 AND period_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 1); // Only one row (idempotent)
}

// ============================================================
// HELPER FUNCTIONS
// ============================================================

/// Cleanup test data for a tenant (delete all periods and related data)
async fn cleanup_test_tenant(pool: &PgPool, tenant_id: &str) {
    // Delete in reverse FK order
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)"
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
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
}

/// Create a test journal entry with two lines (debit and credit)
async fn create_test_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    _period_id: Uuid,
    debit_account_code: &str,
    credit_account_code: &str,
    currency: &str,
    amount_minor: i64,
    entry_date: NaiveDate,
) {

    // Create journal entry (no period_id column in journal_entries table)
    let entry_id = Uuid::new_v4();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test_entry', $3, $4, 'Test entry', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .bind(currency)
    .execute(pool)
    .await
    .unwrap();

    // Create debit line (account_ref is account code, not ID)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES (gen_random_uuid(), $1, 1, $2, $3, 0, 'debit line')
        "#,
    )
    .bind(entry_id)
    .bind(debit_account_code)
    .bind(amount_minor)
    .execute(pool)
    .await
    .unwrap();

    // Create credit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES (gen_random_uuid(), $1, 2, $2, 0, $3, 'credit line')
        "#,
    )
    .bind(entry_id)
    .bind(credit_account_code)
    .bind(amount_minor)
    .execute(pool)
    .await
    .unwrap();
}

/// Get the balance row count for a period (account_balances may not exist for test-only entries)
async fn get_balance_count(pool: &PgPool, tenant_id: &str, period_id: Uuid) -> i64 {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM account_balances
        WHERE tenant_id = $1 AND period_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(pool)
    .await
    .unwrap()
}
