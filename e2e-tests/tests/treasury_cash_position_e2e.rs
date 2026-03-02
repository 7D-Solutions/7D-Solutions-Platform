//! E2E: Treasury cash position — bank accounts + transactions → daily cash position report (bd-1iqz)
//!
//! Proves the treasury cash position report correctly aggregates bank account
//! balances with inflows and outflows against a real PostgreSQL database.
//!
//! Steps:
//! 1. Create bank account with opening balance (via bank statement)
//! 2. Record credit transaction (+250,000 cents — customer payment)
//! 3. Record debit transaction (−75,000 cents — vendor payment)
//! 4. Verify cash position: balance = 1,175,000 cents
//! 5. Verify arithmetic: inflow_total, outflow_total, net_change
//! 6. Verify tenant isolation: other tenant sees no data
//!
//! No mocks, no stubs — all tests run against real treasury PostgreSQL (port 5444).

mod common;

use chrono::NaiveDate;
use common::{generate_test_tenant, wait_for_db_ready};
use sqlx::PgPool;
use uuid::Uuid;

use treasury::domain::accounts::{service as account_svc, CreateBankAccountRequest};
use treasury::domain::reports::cash_position::get_cash_position;
use treasury::domain::txns::{models::InsertBankTxnRequest, service::insert_bank_txn_tx};

// ============================================================================
// Infrastructure
// ============================================================================

fn treasury_db_url() -> String {
    std::env::var("TREASURY_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    })
}

async fn treasury_pool() -> PgPool {
    wait_for_db_ready("treasury", &treasury_db_url()).await
}

const MIGRATION_LOCK_KEY: i64 = 7_831_294_766_i64;

async fn ensure_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory lock failed");

    let migrations = [
        include_str!("../../modules/treasury/db/migrations/20260218000001_create_treasury_schema.sql"),
        include_str!("../../modules/treasury/db/migrations/20260218000002_create_treasury_outbox_idempotency.sql"),
        include_str!("../../modules/treasury/db/migrations/20260218000003_add_credit_card_account_type.sql"),
        include_str!("../../modules/treasury/db/migrations/20260218000004_add_statement_hash.sql"),
        include_str!("../../modules/treasury/db/migrations/20260218000005_add_recon_statement_line.sql"),
    ];
    for sql in migrations {
        let _ = sqlx::raw_sql(sql).execute(pool).await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory unlock failed");
}

async fn cleanup(pool: &PgPool, tenant: &str) {
    sqlx::query("DELETE FROM treasury_recon_matches WHERE app_id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_statements WHERE app_id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'bank_account' AND aggregate_id IN \
         (SELECT id::TEXT FROM treasury_bank_accounts WHERE app_id = $1)",
    )
    .bind(tenant)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .ok();
}

/// Insert a bank statement to establish the opening balance for an account.
async fn insert_opening_balance(pool: &PgPool, tenant: &str, account_id: Uuid, opening_minor: i64) {
    sqlx::query(
        r#"
        INSERT INTO treasury_bank_statements
            (app_id, account_id, period_start, period_end,
             opening_balance_minor, closing_balance_minor, currency, status)
        VALUES ($1, $2, '2026-01-01', '2026-01-31', $3, $3, 'USD',
                'reconciled'::treasury_statement_status)
        "#,
    )
    .bind(tenant)
    .bind(account_id)
    .bind(opening_minor)
    .execute(pool)
    .await
    .expect("insert opening balance statement failed");
}

/// Insert a single transaction using a one-shot db transaction.
async fn insert_txn(
    pool: &PgPool,
    tenant: &str,
    account_id: Uuid,
    amount_minor: i64,
    description: &str,
    external_id: &str,
) {
    let mut tx = pool.begin().await.expect("begin tx failed");
    let req = InsertBankTxnRequest {
        app_id: tenant.to_string(),
        account_id,
        amount_minor,
        currency: "USD".to_string(),
        transaction_date: NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
        description: Some(description.to_string()),
        reference: None,
        external_id: external_id.to_string(),
        auth_date: None,
        settle_date: None,
        merchant_name: None,
        merchant_category_code: None,
    };
    insert_bank_txn_tx(&mut tx, &req)
        .await
        .expect("insert txn failed");
    tx.commit().await.expect("commit failed");
}

// ============================================================================
// Tests
// ============================================================================

/// Core invariant: opening balance + inflow − outflow = current balance.
///
/// Opening: 1,000,000 cents
/// Inflow:    250,000 cents (customer payment)
/// Outflow:    75,000 cents (vendor payment → stored as −75,000)
/// Expected:  1,175,000 cents
#[tokio::test]
async fn test_cash_position_balance_arithmetic() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    // 1. Create bank account
    let account = account_svc::create_bank_account(
        &pool,
        &tenant,
        &CreateBankAccountRequest {
            account_name: "Operating Account".to_string(),
            institution: Some("First National".to_string()),
            account_number_last4: Some("1234".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        },
        None,
        "cp-e2e-corr".to_string(),
    )
    .await
    .expect("create bank account failed");

    // 2. Set opening balance via bank statement (1,000,000 cents = $10,000.00)
    insert_opening_balance(&pool, &tenant, account.id, 1_000_000).await;

    // 3. Record inflow: +250,000 cents (customer payment)
    insert_txn(
        &pool,
        &tenant,
        account.id,
        250_000,
        "Customer payment",
        "cp-in-001",
    )
    .await;

    // 4. Record outflow: −75,000 cents (vendor payment)
    insert_txn(
        &pool,
        &tenant,
        account.id,
        -75_000,
        "Vendor payment",
        "cp-out-001",
    )
    .await;

    // 5. Get cash position report
    let pos = get_cash_position(&pool, &tenant)
        .await
        .expect("cash position query failed");

    // 6. Verify account breakdown
    assert_eq!(pos.bank_cash.len(), 1, "one bank account in report");
    assert!(pos.credit_card_liability.is_empty(), "no CC accounts");

    let acct = &pos.bank_cash[0];
    assert_eq!(acct.account_id, account.id, "correct account in report");
    assert_eq!(acct.account_name, "Operating Account");
    assert_eq!(acct.currency, "USD");
    assert_eq!(
        acct.opening_balance_minor, 1_000_000,
        "opening balance preserved"
    );

    // 7. Verify inflow_total, outflow_total, net_change via DB queries
    let inflow_total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM treasury_bank_transactions \
         WHERE app_id = $1 AND account_id = $2 AND amount_minor > 0",
    )
    .bind(&tenant)
    .bind(account.id)
    .fetch_one(&pool)
    .await
    .expect("inflow query failed");

    let outflow_abs: i64 = sqlx::query_scalar(
        "SELECT COALESCE(ABS(SUM(amount_minor)), 0)::BIGINT FROM treasury_bank_transactions \
         WHERE app_id = $1 AND account_id = $2 AND amount_minor < 0",
    )
    .bind(&tenant)
    .bind(account.id)
    .fetch_one(&pool)
    .await
    .expect("outflow query failed");

    assert_eq!(inflow_total, 250_000, "inflow_total = 250,000 cents");
    assert_eq!(outflow_abs, 75_000, "outflow_total = 75,000 cents");

    let net_change = inflow_total - outflow_abs;
    assert_eq!(
        net_change, 175_000,
        "net_change = inflow − outflow = 175,000"
    );
    assert_eq!(
        acct.transaction_total_minor, net_change,
        "transaction_total_minor matches net_change"
    );

    // 8. Verify final balance: 1,000,000 + 175,000 = 1,175,000
    assert_eq!(
        acct.balance_minor, 1_175_000,
        "balance = opening(1,000,000) + inflow(250,000) − outflow(75,000) = 1,175,000"
    );
    assert_eq!(pos.summary.total_bank_cash_minor, 1_175_000);
    assert_eq!(pos.summary.total_cc_liability_minor, 0);
    assert_eq!(pos.summary.net_position_minor, 1_175_000);
    assert_eq!(pos.summary.currencies, vec!["USD"]);

    cleanup(&pool, &tenant).await;
}

/// Tenant isolation: cash position only includes accounts for the querying tenant.
#[tokio::test]
async fn test_cash_position_tenant_isolation() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;

    let tenant_a = generate_test_tenant();
    let tenant_b = generate_test_tenant();
    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;

    // Create account and transactions for tenant A
    let acct_a = account_svc::create_bank_account(
        &pool,
        &tenant_a,
        &CreateBankAccountRequest {
            account_name: "Tenant A Account".to_string(),
            institution: None,
            account_number_last4: None,
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        },
        None,
        "iso-corr-a".to_string(),
    )
    .await
    .expect("create tenant A account failed");

    insert_opening_balance(&pool, &tenant_a, acct_a.id, 500_000).await;
    insert_txn(
        &pool,
        &tenant_a,
        acct_a.id,
        100_000,
        "A inflow",
        "iso-a-001",
    )
    .await;

    // Tenant B queries — should see nothing
    let pos_b = get_cash_position(&pool, &tenant_b)
        .await
        .expect("tenant B cash position failed");

    assert!(pos_b.bank_cash.is_empty(), "tenant B sees no bank accounts");
    assert_eq!(
        pos_b.summary.net_position_minor, 0,
        "tenant B net position = 0"
    );

    // Tenant A queries — should only see their own account
    let pos_a = get_cash_position(&pool, &tenant_a)
        .await
        .expect("tenant A cash position failed");

    assert_eq!(pos_a.bank_cash.len(), 1, "tenant A sees exactly 1 account");
    assert_eq!(
        pos_a.bank_cash[0].balance_minor, 600_000,
        "tenant A balance = opening(500,000) + inflow(100,000)"
    );

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

/// Precision: cent-level amounts survive round-trip with no rounding loss.
/// Financial integrity invariant: amount_cents must be exact i64 — no floating point.
#[tokio::test]
async fn test_cash_position_precision_no_rounding_loss() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    let account = account_svc::create_bank_account(
        &pool,
        &tenant,
        &CreateBankAccountRequest {
            account_name: "Precision Account".to_string(),
            institution: None,
            account_number_last4: None,
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        },
        None,
        "prec-corr".to_string(),
    )
    .await
    .expect("create account failed");

    // Opening: $12,345.67 = 1,234,567 cents
    insert_opening_balance(&pool, &tenant, account.id, 1_234_567).await;

    // Transactions with non-round cent amounts
    insert_txn(&pool, &tenant, account.id, 99_901, "Inflow A", "prec-001").await; // $999.01
    insert_txn(&pool, &tenant, account.id, -33_333, "Outflow B", "prec-002").await; // −$333.33
    insert_txn(&pool, &tenant, account.id, 1, "Micro inflow", "prec-003").await; // $0.01

    let pos = get_cash_position(&pool, &tenant)
        .await
        .expect("precision cash position failed");

    assert_eq!(pos.bank_cash.len(), 1);
    let acct = &pos.bank_cash[0];

    // net txn = 99901 − 33333 + 1 = 66569
    assert_eq!(
        acct.transaction_total_minor, 66_569,
        "net transactions exact"
    );
    // balance = 1234567 + 66569 = 1301136
    assert_eq!(
        acct.balance_minor, 1_301_136,
        "balance exact — no rounding loss"
    );
    assert_eq!(pos.summary.net_position_minor, 1_301_136);

    cleanup(&pool, &tenant).await;
}
