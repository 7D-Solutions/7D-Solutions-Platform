//! Integrated tests for the reconciliation service.

use super::*;
use crate::domain::accounts::{service as acct_svc, CreateBankAccountRequest};
use crate::domain::txns::models::InsertBankTxnRequest;
use crate::domain::txns::service as txn_svc;
use chrono::NaiveDate;
use serial_test::serial;

const TEST_APP: &str = "test-app-recon";

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
    sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
        .bind(TEST_APP)
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
        account_name: "Recon Test Account".to_string(),
        institution: Some("Test Bank".to_string()),
        account_number_last4: Some("8888".to_string()),
        routing_number: None,
        currency: "USD".to_string(),
        metadata: None,
    };
    acct_svc::create_bank_account(pool, TEST_APP, &req, None, "test".to_string())
        .await
        .expect("create test account")
        .id
}

/// Insert a statement line (has statement_id).
async fn insert_statement_line(
    pool: &PgPool,
    account_id: Uuid,
    amount: i64,
    date: NaiveDate,
    reference: Option<&str>,
) -> Uuid {
    let stmt_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO treasury_bank_statements
           (id, app_id, account_id, period_start, period_end,
            opening_balance_minor, closing_balance_minor, currency,
            status, imported_at, statement_hash, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $4, 0, 0, 'USD',
                   'imported'::treasury_statement_status, NOW(),
                   gen_random_uuid(), NOW(), NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(stmt_id)
    .bind(TEST_APP)
    .bind(account_id)
    .bind(date)
    .execute(pool)
    .await
    .unwrap();

    let txn_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (id, app_id, account_id, statement_id, transaction_date,
            amount_minor, currency, description, reference, external_id)
           VALUES ($1, $2, $3, $4, $5, $6, 'USD', 'stmt line', $7, $8)"#,
    )
    .bind(txn_id)
    .bind(TEST_APP)
    .bind(account_id)
    .bind(stmt_id)
    .bind(date)
    .bind(amount)
    .bind(reference)
    .bind(format!("stmt:{}:line:0", stmt_id))
    .execute(pool)
    .await
    .unwrap();

    txn_id
}

/// Insert a payment-event transaction (no statement_id).
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
        external_id: format!("pay:{}", Uuid::new_v4()),
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

#[tokio::test]
#[serial]
async fn test_auto_match_exact() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    insert_statement_line(&pool, acct, -450, d, Some("TXN001")).await;
    insert_payment_txn(&pool, acct, -450, d, Some("TXN001")).await;

    let result = run_auto_match(&pool, TEST_APP, acct, "test-corr")
        .await
        .unwrap();
    assert_eq!(result.matches_created, 1);
    assert_eq!(result.unmatched_statement_lines, 0);
    assert_eq!(result.unmatched_transactions, 0);

    // Running again should find nothing new (idempotent)
    let result2 = run_auto_match(&pool, TEST_APP, acct, "test-corr2")
        .await
        .unwrap();
    assert_eq!(result2.matches_created, 0);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_manual_match_creates_record() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 2, 10).unwrap();

    let sl_id = insert_statement_line(&pool, acct, -1000, d, None).await;
    let pt_id = insert_payment_txn(&pool, acct, -999, d, None).await;

    let req = ManualMatchRequest {
        statement_line_id: sl_id,
        bank_transaction_id: pt_id,
    };
    let m = create_manual_match(&pool, TEST_APP, &req, "tester", "corr1")
        .await
        .unwrap();

    assert_eq!(m.statement_line_id, Some(sl_id));
    assert_eq!(m.bank_transaction_id, pt_id);
    assert_eq!(m.match_type, ReconMatchType::Manual);
    assert!(m.superseded_by.is_none());

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_manual_rematch_supersedes_old() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let sl_id = insert_statement_line(&pool, acct, -500, d, None).await;
    let pt1_id = insert_payment_txn(&pool, acct, -500, d, Some("WRONG")).await;
    let pt2_id = insert_payment_txn(&pool, acct, -500, d, Some("RIGHT")).await;

    // First match
    let req1 = ManualMatchRequest {
        statement_line_id: sl_id,
        bank_transaction_id: pt1_id,
    };
    let m1 = create_manual_match(&pool, TEST_APP, &req1, "tester", "corr1")
        .await
        .unwrap();

    // Rematch to different transaction
    let req2 = ManualMatchRequest {
        statement_line_id: sl_id,
        bank_transaction_id: pt2_id,
    };
    let m2 = create_manual_match(&pool, TEST_APP, &req2, "tester", "corr2")
        .await
        .unwrap();

    assert_ne!(m1.id, m2.id);
    assert!(m2.superseded_by.is_none(), "new match is active");

    // Old match should be superseded
    let old = fetch_match(&pool, m1.id).await.unwrap().unwrap();
    assert!(old.superseded_by.is_some(), "old match must be superseded");
    assert_eq!(old.status, ReconMatchStatus::Rejected);

    // Old transaction should be unmatched again
    let old_txn_status: String =
        sqlx::query_scalar("SELECT status::text FROM treasury_bank_transactions WHERE id = $1")
            .bind(pt1_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(old_txn_status, "unmatched");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_list_matches_excludes_superseded() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let acct = create_test_account(&pool).await;
    let d = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let sl_id = insert_statement_line(&pool, acct, -200, d, None).await;
    let pt1_id = insert_payment_txn(&pool, acct, -200, d, None).await;
    let pt2_id = insert_payment_txn(&pool, acct, -200, d, None).await;

    let req1 = ManualMatchRequest {
        statement_line_id: sl_id,
        bank_transaction_id: pt1_id,
    };
    create_manual_match(&pool, TEST_APP, &req1, "t", "c1")
        .await
        .unwrap();

    let req2 = ManualMatchRequest {
        statement_line_id: sl_id,
        bank_transaction_id: pt2_id,
    };
    create_manual_match(&pool, TEST_APP, &req2, "t", "c2")
        .await
        .unwrap();

    // Default: exclude superseded
    let active = list_matches(&pool, TEST_APP, acct, false).await.unwrap();
    assert_eq!(active.len(), 1);

    // Include superseded
    let all = list_matches(&pool, TEST_APP, acct, true).await.unwrap();
    assert_eq!(all.len(), 2);

    cleanup(&pool).await;
}
