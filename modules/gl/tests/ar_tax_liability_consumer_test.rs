//! Integration tests for the AR tax liability GL consumer (bd-3gsz)
//!
//! Tests `process_tax_committed_posting` and `process_tax_voided_posting` directly
//! against the real GL database. No NATS, no mocks, no stubs.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - PostgreSQL at localhost:5438 (GL DB)

mod common;

use chrono::Utc;
use common::{get_test_pool, setup_test_account, setup_test_period};
use gl_rs::consumers::ar_tax_liability::{
    process_tax_committed_posting, process_tax_voided_posting, TaxCommittedPayload,
    TaxVoidedPayload, TAX_COLLECTED_ACCOUNT, TAX_PAYABLE_ACCOUNT,
};
use serial_test::serial;
use uuid::Uuid;

const TEST_TENANT: &str = "test-tenant-tax-liability";

/// Setup required GL accounts and an open period for the test tenant.
async fn setup_gl_env(pool: &sqlx::PgPool) {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let period_end = chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
    setup_test_period(pool, TEST_TENANT, today, period_end).await;

    for (code, name, acct_type, normal) in [
        (TAX_COLLECTED_ACCOUNT, "Tax Collected", "asset", "debit"),
        (
            TAX_PAYABLE_ACCOUNT,
            "Sales Tax Payable",
            "liability",
            "credit",
        ),
    ] {
        setup_test_account(pool, TEST_TENANT, code, name, acct_type, normal).await;
    }
}

/// Cleanup test data in FK-safe order.
async fn cleanup(pool: &sqlx::PgPool) {
    for q in [
        "DELETE FROM processed_events WHERE tenant_id = $1",
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM account_balances WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
    }
}

fn make_committed_payload(invoice_id: &str, tax_minor: i64) -> TaxCommittedPayload {
    TaxCommittedPayload {
        tenant_id: TEST_TENANT.to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: "cust-tax-test".to_string(),
        total_tax_minor: tax_minor,
        currency: "USD".to_string(),
        provider_quote_ref: format!("quote-{}", Uuid::new_v4()),
        provider_commit_ref: format!("commit-{}", Uuid::new_v4()),
        provider: "local".to_string(),
        committed_at: Utc::now(),
    }
}

fn make_voided_payload(invoice_id: &str, tax_minor: i64, reason: &str) -> TaxVoidedPayload {
    TaxVoidedPayload {
        tenant_id: TEST_TENANT.to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: "cust-tax-test".to_string(),
        total_tax_minor: tax_minor,
        currency: "USD".to_string(),
        provider_commit_ref: format!("commit-{}", Uuid::new_v4()),
        provider: "local".to_string(),
        void_reason: reason.to_string(),
        voided_at: Utc::now(),
    }
}

// ============================================================================
// tax.committed tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tax_committed_posts_balanced_liability_entry() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = make_committed_payload("inv-tax-001", 850); // $8.50 tax

    let entry_id = process_tax_committed_posting(&pool, event_id, TEST_TENANT, "ar", &payload)
        .await
        .expect("tax committed posting should succeed");

    // Verify journal entry exists
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE id = $1 AND tenant_id = $2")
            .bind(entry_id)
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("query journal_entries");
    assert_eq!(count, 1, "journal entry should exist");

    // Verify balanced: DR TAX_COLLECTED, CR TAX_PAYABLE
    let (total_dr, total_cr): (i64, i64) = sqlx::query_as(
        "SELECT SUM(debit_minor)::bigint, SUM(credit_minor)::bigint \
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("sum journal_lines");
    assert_eq!(total_dr, total_cr, "journal must balance");
    assert_eq!(
        total_dr, 850,
        "debit should equal tax amount in minor units"
    );

    // Verify account references
    let debit_account: (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND debit_minor > 0",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("fetch debit line");
    assert_eq!(debit_account.0, TAX_COLLECTED_ACCOUNT);

    let credit_account: (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND credit_minor > 0",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("fetch credit line");
    assert_eq!(credit_account.0, TAX_PAYABLE_ACCOUNT);

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_tax_committed_idempotent_no_duplicate() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = make_committed_payload("inv-tax-idem", 500);

    // First posting succeeds
    process_tax_committed_posting(&pool, event_id, TEST_TENANT, "ar", &payload)
        .await
        .expect("first posting");

    // Second posting with same event_id returns DuplicateEvent
    let second = process_tax_committed_posting(&pool, event_id, TEST_TENANT, "ar", &payload).await;
    assert!(
        matches!(
            second,
            Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))
        ),
        "second posting must return DuplicateEvent"
    );

    // Only one journal entry
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(TEST_TENANT)
    .bind("inv-tax-idem")
    .fetch_one(&pool)
    .await
    .expect("count entries");
    assert_eq!(count, 1, "only one journal entry for same event_id");

    cleanup(&pool).await;
    pool.close().await;
}

// ============================================================================
// tax.voided tests (reversal path)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tax_voided_posts_reversal_entry() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = make_voided_payload("inv-tax-void-001", 850, "invoice_cancelled");

    let entry_id = process_tax_voided_posting(&pool, event_id, TEST_TENANT, "ar", &payload)
        .await
        .expect("tax voided posting should succeed");

    // Verify balanced: DR TAX_PAYABLE (reversal), CR TAX_COLLECTED (reversal)
    let (total_dr, total_cr): (i64, i64) = sqlx::query_as(
        "SELECT SUM(debit_minor)::bigint, SUM(credit_minor)::bigint \
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("sum journal_lines");
    assert_eq!(total_dr, total_cr, "reversal journal must balance");
    assert_eq!(total_dr, 850, "reversal debit should equal voided tax");

    // Verify accounts are reversed (DR TAX_PAYABLE, CR TAX_COLLECTED)
    let debit_account: (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND debit_minor > 0",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("fetch debit line");
    assert_eq!(
        debit_account.0, TAX_PAYABLE_ACCOUNT,
        "void debits TAX_PAYABLE"
    );

    let credit_account: (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND credit_minor > 0",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("fetch credit line");
    assert_eq!(
        credit_account.0, TAX_COLLECTED_ACCOUNT,
        "void credits TAX_COLLECTED"
    );

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_tax_committed_then_voided_nets_to_zero() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let invoice_id = "inv-tax-net-zero";

    // Step 1: Commit tax
    let commit_event = Uuid::new_v4();
    let commit_payload = make_committed_payload(invoice_id, 1200); // $12.00
    process_tax_committed_posting(&pool, commit_event, TEST_TENANT, "ar", &commit_payload)
        .await
        .expect("commit posting");

    // Step 2: Void the same tax
    let void_event = Uuid::new_v4();
    let void_payload = make_voided_payload(invoice_id, 1200, "full_refund");
    process_tax_voided_posting(&pool, void_event, TEST_TENANT, "ar", &void_payload)
        .await
        .expect("void posting");

    // Verify: net balance on TAX_PAYABLE should be zero
    // Committed: CR TAX_PAYABLE 1200, Voided: DR TAX_PAYABLE 1200 → net 0
    let (net_dr, net_cr): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(jl.debit_minor), 0)::bigint, \
                COALESCE(SUM(jl.credit_minor), 0)::bigint \
         FROM journal_lines jl \
         JOIN journal_entries je ON je.id = jl.journal_entry_id \
         WHERE je.tenant_id = $1 AND jl.account_ref = $2",
    )
    .bind(TEST_TENANT)
    .bind(TAX_PAYABLE_ACCOUNT)
    .fetch_one(&pool)
    .await
    .expect("net tax payable");
    assert_eq!(net_dr, net_cr, "TAX_PAYABLE nets to zero after commit+void");

    // Same for TAX_COLLECTED
    let (net_dr2, net_cr2): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(jl.debit_minor), 0)::bigint, \
                COALESCE(SUM(jl.credit_minor), 0)::bigint \
         FROM journal_lines jl \
         JOIN journal_entries je ON je.id = jl.journal_entry_id \
         WHERE je.tenant_id = $1 AND jl.account_ref = $2",
    )
    .bind(TEST_TENANT)
    .bind(TAX_COLLECTED_ACCOUNT)
    .fetch_one(&pool)
    .await
    .expect("net tax collected");
    assert_eq!(
        net_dr2, net_cr2,
        "TAX_COLLECTED nets to zero after commit+void"
    );

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_tax_committed_rejects_closed_period() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;

    // Create a CLOSED period
    let period_start = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let period_end = chrono::NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
    let period_id = setup_test_period(&pool, TEST_TENANT, period_start, period_end).await;

    // Close the period (close_hash required by DB constraint)
    sqlx::query(
        "UPDATE accounting_periods SET closed_at = NOW(), close_hash = 'test_hash' WHERE id = $1",
    )
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("close period");

    // Setup accounts
    for (code, name, acct_type, normal) in [
        (TAX_COLLECTED_ACCOUNT, "Tax Collected", "asset", "debit"),
        (
            TAX_PAYABLE_ACCOUNT,
            "Sales Tax Payable",
            "liability",
            "credit",
        ),
    ] {
        setup_test_account(&pool, TEST_TENANT, code, name, acct_type, normal).await;
    }

    // Try posting into the closed period
    let event_id = Uuid::new_v4();
    let mut payload = make_committed_payload("inv-closed-period", 500);
    payload.committed_at = chrono::DateTime::from_naive_utc_and_offset(
        period_start.and_hms_opt(12, 0, 0).unwrap(),
        Utc,
    );

    let result = process_tax_committed_posting(&pool, event_id, TEST_TENANT, "ar", &payload).await;
    assert!(
        matches!(
            result,
            Err(gl_rs::services::journal_service::JournalError::Period(_))
        ),
        "posting to closed period must return Period error, got: {:?}",
        result
    );

    cleanup(&pool).await;
    pool.close().await;
}
