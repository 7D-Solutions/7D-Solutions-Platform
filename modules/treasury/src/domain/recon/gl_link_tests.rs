//! Integrated tests for GL reconciliation linkage.

use super::*;
use crate::domain::accounts::{service as acct_svc, CreateBankAccountRequest};
use crate::domain::recon::models::{ReconMatchStatus, ReconMatchType};
use crate::domain::recon::service as recon_svc;
use crate::domain::txns::models::InsertBankTxnRequest;
use crate::domain::txns::service as txn_svc;
use chrono::NaiveDate;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_APP: &str = "test-app-gl-link";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    })
}

async fn test_pool() -> PgPool {
    sqlx::PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to treasury test database")
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM treasury_recon_matches WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_statements WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type IN ('recon', 'bank_account', 'bank_statement')",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
}

async fn create_test_account(pool: &PgPool) -> Uuid {
    let req = CreateBankAccountRequest {
        account_name: "GL Link Test Account".to_string(),
        institution: Some("Test Bank".to_string()),
        account_number_last4: Some("7777".to_string()),
        routing_number: None,
        currency: "USD".to_string(),
        metadata: None,
    };
    acct_svc::create_bank_account(pool, TEST_APP, &req, None, "test".to_string())
        .await
        .expect("create test account")
        .id
}

async fn insert_payment_txn(
    pool: &PgPool,
    account_id: Uuid,
    amount: i64,
    date: NaiveDate,
    reference: Option<&str>,
) -> Uuid {
    let req = InsertBankTxnRequest {
        app_id: TEST_APP.to_string(),
        account_id,
        amount_minor: amount,
        currency: "USD".to_string(),
        transaction_date: date,
        description: Some("payment event".to_string()),
        reference: reference.map(String::from),
        external_id: format!("gl-pay:{}", Uuid::new_v4()),
        auth_date: None,
        settle_date: None,
        merchant_name: None,
        merchant_category_code: None,
    };
    let mut tx = pool.begin().await.unwrap();
    txn_svc::insert_bank_txn_tx(&mut tx, &req).await.unwrap();
    tx.commit().await.unwrap();

    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM treasury_bank_transactions WHERE external_id = $1",
    )
    .bind(&req.external_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_link_creates_gl_only_match() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 5, 1).unwrap();
    let txn_id = insert_payment_txn(&pool, acct, -750, d, Some("GL001")).await;

    let req = LinkToGlRequest {
        bank_transaction_id: txn_id,
        gl_entry_id: 42001,
    };
    let m = link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr1")
        .await
        .unwrap();

    assert_eq!(m.bank_transaction_id, txn_id);
    assert_eq!(m.gl_entry_id, Some(42001));
    assert_eq!(m.status, ReconMatchStatus::Confirmed);
    assert!(m.statement_line_id.is_none(), "GL-only match has no statement line");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_link_is_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 5, 2).unwrap();
    let txn_id = insert_payment_txn(&pool, acct, -300, d, None).await;

    let req = LinkToGlRequest {
        bank_transaction_id: txn_id,
        gl_entry_id: 42002,
    };

    let m1 = link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr1")
        .await
        .unwrap();
    let m2 = link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr2")
        .await
        .unwrap();

    assert_eq!(m1.id, m2.id, "same match returned on replay");

    // Verify only one match row exists
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_recon_matches WHERE bank_transaction_id = $1 AND app_id = $2",
    )
    .bind(txn_id)
    .bind(TEST_APP)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_link_updates_existing_match_without_gl() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 5, 3).unwrap();

    // Insert statement line and payment txn, then auto-match them
    let stmt_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO treasury_bank_statements
           (id, app_id, account_id, period_start, period_end,
            opening_balance_minor, closing_balance_minor, currency,
            status, imported_at, statement_hash, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $4, 0, 0, 'USD',
                   'imported'::treasury_statement_status, NOW(),
                   gen_random_uuid(), NOW(), NOW())"#,
    )
    .bind(stmt_id)
    .bind(TEST_APP)
    .bind(acct)
    .bind(d)
    .execute(&pool)
    .await
    .unwrap();

    let sl_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (id, app_id, account_id, statement_id, transaction_date,
            amount_minor, currency, description, reference, external_id)
           VALUES ($1, $2, $3, $4, $5, $6, 'USD', 'stmt line', $7, $8)"#,
    )
    .bind(sl_id)
    .bind(TEST_APP)
    .bind(acct)
    .bind(stmt_id)
    .bind(d)
    .bind(-500_i64)
    .bind(Some("REF123"))
    .bind(format!("stmt:{}:line:0", stmt_id))
    .execute(&pool)
    .await
    .unwrap();

    let pay_id = insert_payment_txn(&pool, acct, -500, d, Some("REF123")).await;

    // Auto-match creates a statement-line<->bank-txn match (no GL link)
    let result = recon_svc::run_auto_match(&pool, TEST_APP, acct, "test-corr")
        .await
        .unwrap();
    assert_eq!(result.matches_created, 1);

    // Now link the payment txn to a GL entry — should UPDATE the existing match
    let req = LinkToGlRequest {
        bank_transaction_id: pay_id,
        gl_entry_id: 42003,
    };
    let m = link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr-gl")
        .await
        .unwrap();

    assert_eq!(m.gl_entry_id, Some(42003));
    assert_eq!(m.bank_transaction_id, pay_id);
    // Should still be the same match row (updated, not a new one)
    assert_eq!(m.match_type, ReconMatchType::Auto);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_link_rejects_nonexistent_txn() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = LinkToGlRequest {
        bank_transaction_id: Uuid::new_v4(),
        gl_entry_id: 99999,
    };
    let err = link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr1")
        .await
        .unwrap_err();

    assert!(
        matches!(err, crate::domain::recon::ReconError::TransactionNotFound(_)),
        "expected TransactionNotFound, got: {:?}",
        err
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_unmatched_bank_txns_for_gl() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();

    let txn1 = insert_payment_txn(&pool, acct, -100, d, None).await;
    let txn2 = insert_payment_txn(&pool, acct, -200, d, None).await;
    let _txn3 = insert_payment_txn(&pool, acct, -300, d, None).await;

    // Link txn1 to GL — it should NOT appear in unmatched
    let req = LinkToGlRequest {
        bank_transaction_id: txn1,
        gl_entry_id: 50001,
    };
    link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr1")
        .await
        .unwrap();

    let unmatched = unmatched_bank_txns_for_gl(&pool, TEST_APP, acct)
        .await
        .unwrap();

    // txn2 and txn3 should be unmatched
    assert_eq!(unmatched.len(), 2);
    let ids: Vec<Uuid> = unmatched.iter().map(|t| t.id).collect();
    assert!(!ids.contains(&txn1), "linked txn should not appear");
    assert!(ids.contains(&txn2), "unlinked txn2 should appear");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_unmatched_gl_entries() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 7, 1).unwrap();

    let txn1 = insert_payment_txn(&pool, acct, -400, d, None).await;

    // Link txn1 to GL entry 60001
    let req = LinkToGlRequest {
        bank_transaction_id: txn1,
        gl_entry_id: 60001,
    };
    link_bank_txn_to_gl(&pool, TEST_APP, &req, "tester", "corr1")
        .await
        .unwrap();

    // Query: which of these GL entries are unmatched?
    let result = unmatched_gl_entries(&pool, TEST_APP, &[60001, 60002, 60003])
        .await
        .unwrap();

    assert_eq!(result.provided, 3);
    assert_eq!(result.linked, 1);
    assert_eq!(result.unmatched_gl_entry_ids.len(), 2);
    assert!(result.unmatched_gl_entry_ids.contains(&60002));
    assert!(result.unmatched_gl_entry_ids.contains(&60003));
    assert!(!result.unmatched_gl_entry_ids.contains(&60001));

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_unmatched_gl_entries_empty_input() {
    let pool = test_pool().await;

    let result = unmatched_gl_entries(&pool, TEST_APP, &[])
        .await
        .unwrap();

    assert_eq!(result.provided, 0);
    assert_eq!(result.linked, 0);
    assert!(result.unmatched_gl_entry_ids.is_empty());
}
