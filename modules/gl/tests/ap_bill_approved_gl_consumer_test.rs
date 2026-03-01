//! Integration tests for the AP vendor_bill_approved GL consumer (bd-1l86)
//!
//! Tests `process_ap_bill_approved_posting` directly against the real GL database.
//! No NATS, no mocks, no stubs.
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - PostgreSQL at localhost:5438 (GL DB)

mod common;

use chrono::Utc;
use common::{get_test_pool, setup_test_account, setup_test_period};
use gl_rs::consumers::ap_vendor_bill_approved_consumer::{
    process_ap_bill_approved_posting, ApprovedGlLine, VendorBillApprovedPayload,
    AP_CLEARING_ACCOUNT,
};
use serial_test::serial;
use uuid::Uuid;

const TEST_TENANT: &str = "test-tenant-ap-bill-gl";

/// Setup all required accounts and an open period for the test tenant.
async fn setup_gl_env(pool: &sqlx::PgPool) {
    // Open accounting period covering today
    let today = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let period_end = chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
    setup_test_period(pool, TEST_TENANT, today, period_end).await;

    // Required GL accounts
    for (code, name, acct_type, normal) in [
        ("AP", "Accounts Payable", "liability", "credit"),
        ("AP_CLEARING", "AP Clearing", "liability", "credit"),
        ("EXPENSE", "Default Expense", "expense", "debit"),
        ("6100", "Widget Expense", "expense", "debit"),
        ("6200", "Consulting Expense", "expense", "debit"),
    ] {
        setup_test_account(pool, TEST_TENANT, code, name, acct_type, normal).await;
    }
}

/// Cleanup test data in FK-safe order.
async fn cleanup(pool: &sqlx::PgPool) {
    for q in [
        "DELETE FROM processed_events WHERE id IN \
         (SELECT id FROM processed_events WHERE tenant_id = $1)",
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

fn make_payload(
    bill_id: Uuid,
    amount_minor: i64,
    gl_lines: Vec<ApprovedGlLine>,
    fx_rate_id: Option<Uuid>,
) -> VendorBillApprovedPayload {
    VendorBillApprovedPayload {
        bill_id,
        tenant_id: TEST_TENANT.to_string(),
        vendor_id: Uuid::new_v4(),
        vendor_invoice_ref: format!("INV-{}", &bill_id.to_string()[..8]),
        approved_amount_minor: amount_minor,
        currency: "USD".to_string(),
        due_date: Utc::now(),
        approved_by: "approver-gl-test".to_string(),
        approved_at: Utc::now(),
        fx_rate_id,
        gl_lines,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_ap_bill_approved_single_expense_line_posts_balanced_entry() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let bill_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let gl_lines = vec![ApprovedGlLine {
        line_id: Uuid::new_v4(),
        gl_account_code: "6100".to_string(),
        amount_minor: 50000,
        po_line_id: None,
    }];
    let payload = make_payload(bill_id, 50000, gl_lines, None);

    let result = process_ap_bill_approved_posting(&pool, event_id, TEST_TENANT, "ap", &payload)
        .await
        .expect("posting should succeed");

    // Journal entry was created
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE id = $1 AND tenant_id = $2")
            .bind(result)
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("query journal_entries");
    assert_eq!(count, 1, "journal entry should be created");

    // Lines: DR 6100, CR AP
    let (dr_count, cr_count): (i64, i64) = sqlx::query_as(
        "SELECT \
         COUNT(*) FILTER (WHERE debit_minor > 0 AND credit_minor = 0), \
         COUNT(*) FILTER (WHERE credit_minor > 0 AND debit_minor = 0) \
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(result)
    .fetch_one(&pool)
    .await
    .expect("query journal_lines");
    assert_eq!(dr_count, 1, "one debit line");
    assert_eq!(cr_count, 1, "one credit line");

    // Balance in base currency (SUM of BIGINT returns NUMERIC in PG; cast required)
    let (total_dr, total_cr): (i64, i64) = sqlx::query_as(
        "SELECT SUM(debit_minor)::bigint, SUM(credit_minor)::bigint \
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(result)
    .fetch_one(&pool)
    .await
    .expect("sum journal_lines");
    assert_eq!(total_dr, total_cr, "journal must balance");

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_ap_bill_approved_po_backed_line_posts_to_ap_clearing() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let bill_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    // PO-backed: po_line_id is Some → should go to AP_CLEARING not gl_account_code
    let gl_lines = vec![ApprovedGlLine {
        line_id: Uuid::new_v4(),
        gl_account_code: "6100".to_string(),
        amount_minor: 30000,
        po_line_id: Some(Uuid::new_v4()),
    }];
    let payload = make_payload(bill_id, 30000, gl_lines, None);

    let result = process_ap_bill_approved_posting(&pool, event_id, TEST_TENANT, "ap", &payload)
        .await
        .expect("PO-backed posting should succeed");

    // Debit line should reference AP_CLEARING, not 6100
    let (debit_account,): (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND debit_minor > 0",
    )
    .bind(result)
    .fetch_one(&pool)
    .await
    .expect("fetch debit line");
    assert_eq!(
        debit_account, AP_CLEARING_ACCOUNT,
        "PO-backed: debit should go to AP_CLEARING, not expense account"
    );

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_ap_bill_approved_empty_gl_lines_fallback_to_expense() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let bill_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    // No gl_lines → fallback to DEFAULT_EXPENSE_ACCOUNT
    let payload = make_payload(bill_id, 100_00, vec![], None);

    let result = process_ap_bill_approved_posting(&pool, event_id, TEST_TENANT, "ap", &payload)
        .await
        .expect("fallback posting should succeed");

    // Should have exactly 2 lines (DR EXPENSE, CR AP)
    let (total_lines,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_lines WHERE journal_entry_id = $1")
            .bind(result)
            .fetch_one(&pool)
            .await
            .expect("count lines");
    assert_eq!(
        total_lines, 2,
        "fallback: 2 lines expected (DR EXPENSE + CR AP)"
    );

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_ap_bill_approved_multi_line_bill_all_debit_to_expense_accounts() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let bill_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let gl_lines = vec![
        ApprovedGlLine {
            line_id: Uuid::new_v4(),
            gl_account_code: "6100".to_string(),
            amount_minor: 20000,
            po_line_id: None,
        },
        ApprovedGlLine {
            line_id: Uuid::new_v4(),
            gl_account_code: "6200".to_string(),
            amount_minor: 30000,
            po_line_id: None,
        },
    ];
    let payload = make_payload(bill_id, 50000, gl_lines, None);

    let result = process_ap_bill_approved_posting(&pool, event_id, TEST_TENANT, "ap", &payload)
        .await
        .expect("multi-line posting should succeed");

    // 3 lines: DR 6100, DR 6200, CR AP
    let (total_lines,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_lines WHERE journal_entry_id = $1")
            .bind(result)
            .fetch_one(&pool)
            .await
            .expect("count lines");
    assert_eq!(total_lines, 3, "multi-line: 3 journal lines expected");

    // Total debits == total credits (cast SUM from NUMERIC to BIGINT)
    let (total_dr, total_cr): (i64, i64) = sqlx::query_as(
        "SELECT SUM(debit_minor)::bigint, SUM(credit_minor)::bigint \
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(result)
    .fetch_one(&pool)
    .await
    .expect("sum multi-line");
    assert_eq!(total_dr, total_cr, "multi-line journal must balance");

    cleanup(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_ap_bill_approved_idempotent_no_duplicate_entry() {
    let pool = get_test_pool().await;
    cleanup(&pool).await;
    setup_gl_env(&pool).await;

    let bill_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let gl_lines = vec![ApprovedGlLine {
        line_id: Uuid::new_v4(),
        gl_account_code: "6100".to_string(),
        amount_minor: 50000,
        po_line_id: None,
    }];
    let payload = make_payload(bill_id, 50000, gl_lines, None);

    // First posting
    process_ap_bill_approved_posting(&pool, event_id, TEST_TENANT, "ap", &payload)
        .await
        .expect("first posting");

    // Second posting (same event_id) must return DuplicateEvent, not create a second entry
    let second =
        process_ap_bill_approved_posting(&pool, event_id, TEST_TENANT, "ap", &payload).await;
    assert!(
        matches!(
            second,
            Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))
        ),
        "second posting with same event_id must return DuplicateEvent"
    );

    // Only one journal entry should exist (reference_id = bill_id)
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(TEST_TENANT)
    .bind(bill_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count entries");
    assert_eq!(count, 1, "idempotent: only one journal entry expected");

    cleanup(&pool).await;
    pool.close().await;
}
