//! E2E: CC statement import + txn ingest + CC reconciliation + liability position (bd-2blb)
//!
//! Proves the full CC treasury lifecycle against real PostgreSQL:
//! 1. Create a credit card account
//! 2. Import a Chase-format CC statement CSV (charges negative, payments positive)
//! 3. Ingest normalized CC expense transactions directly (the "payment side" to match)
//! 4. Run auto-match reconciliation (uses CreditCardStrategy automatically)
//! 5. Verify matches produced and unmatched counts
//! 6. Verify cash position reports CC balance as liability — separate from bank cash
//! 7. Idempotent rerun: same call yields 0 new matches, identical cash position
//!
//! No mocks, no stubs — all tests run against real treasury PostgreSQL (port 5444).

mod common;

use chrono::NaiveDate;
use common::{generate_test_tenant, wait_for_db_ready};
use sqlx::PgPool;
use uuid::Uuid;

use treasury::domain::accounts::{
    service as account_svc, CreateBankAccountRequest, CreateCreditCardAccountRequest,
};
use treasury::domain::import::adapters::CsvFormat;
use treasury::domain::import::service::{import_statement, ImportRequest};
use treasury::domain::recon::models::ReconMatchType;
use treasury::domain::recon::service::{list_matches, run_auto_match};
use treasury::domain::reports::cash_position::get_cash_position;

// ============================================================================
// Test infrastructure
// ============================================================================

fn treasury_db_url() -> String {
    std::env::var("TREASURY_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    })
}

async fn treasury_pool() -> PgPool {
    wait_for_db_ready("treasury", &treasury_db_url()).await
}

const MIGRATION_LOCK_KEY: i64 = 7_831_294_766_i64; // distinct from bank recon test

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

async fn cleanup(pool: &PgPool, app_id: &str) {
    // Reverse FK order
    sqlx::query("DELETE FROM treasury_recon_matches WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_statements WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type IN \
         ('bank_account', 'bank_statement', 'recon') AND aggregate_id IN \
         (SELECT id::TEXT FROM treasury_bank_accounts WHERE app_id = $1)",
    )
    .bind(app_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM processed_events WHERE event_id IN \
                 (SELECT id::uuid FROM treasury_bank_transactions WHERE app_id = $1 \
                  AND external_id IS NOT NULL AND \
                  external_id ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

/// Chase-format CC CSV for February 2026.
///
/// Charges (money out) are negative:
/// - 2026-02-01: STARBUCKS          -$15.00  (will match expense txn)
/// - 2026-02-03: AMAZON.COM         -$89.99  (will match expense txn)
/// - 2026-02-05: PAYMENT THANK YOU  +$200.00 (no expense txn counterpart)
/// - 2026-02-10: ANNUAL FEE         -$95.00  (no expense txn counterpart)
fn chase_cc_csv() -> Vec<u8> {
    b"Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
      02/01/2026,02/02/2026,STARBUCKS,Food & Drink,Sale,-15.00,\n\
      02/03/2026,02/04/2026,AMAZON.COM,Shopping,Sale,-89.99,\n\
      02/05/2026,02/05/2026,PAYMENT THANK YOU,,Payment,200.00,\n\
      02/10/2026,02/11/2026,ANNUAL FEE,Fees,Sale,-95.00,\n"
        .to_vec()
}

/// Insert a CC expense transaction directly against the CC account (payment side for recon).
///
/// CC expense transactions have:
/// - Negative amount (money charged to the card)
/// - Optional auth_date / settle_date for precise CC date-window matching
/// - Optional merchant_name for CC strategy merchant scoring
async fn insert_cc_expense_txn(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    amount_minor: i64,
    date: NaiveDate,
    description: &str,
    reference: Option<&str>,
    external_id: &str,
    auth_date: Option<NaiveDate>,
    settle_date: Option<NaiveDate>,
    merchant_name: Option<&str>,
) {
    sqlx::query(
        r#"
        INSERT INTO treasury_bank_transactions
            (app_id, account_id, transaction_date, amount_minor, currency,
             description, reference, external_id,
             auth_date, settle_date, merchant_name)
        VALUES ($1, $2, $3, $4, 'USD', $5, $6, $7, $8, $9, $10)
        ON CONFLICT (account_id, external_id) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .bind(date)
    .bind(amount_minor)
    .bind(description)
    .bind(reference)
    .bind(external_id)
    .bind(auth_date)
    .bind(settle_date)
    .bind(merchant_name)
    .execute(pool)
    .await
    .expect("insert CC expense txn failed");
}

// ============================================================================
// Tests
// ============================================================================

/// Full CC lifecycle: import → ingest expenses → auto-match → cash position → rerun.
///
/// Acceptance criteria:
/// - Chase CSV imports 4 lines successfully
/// - 2 expense txns inserted (Starbucks + Amazon) + 1 unmatched expense (Software)
/// - Auto-match: 2 matches (Starbucks + Amazon via CC amount/date strategy)
/// - Unmatched: 2 statement lines (PAYMENT THANK YOU + ANNUAL FEE), 1 expense (Software)
/// - Cash position: CC account in credit_card_liability bucket, bank_cash empty
/// - Idempotent rerun: 0 new matches, same cash position
#[tokio::test]
async fn test_cc_recon_full_lifecycle() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    // --- Step 1: Create CC account ---
    let cc = account_svc::create_credit_card_account(
        &pool,
        &tenant,
        &CreateCreditCardAccountRequest {
            account_name: "Corporate Visa".to_string(),
            institution: Some("Chase".to_string()),
            account_number_last4: Some("4321".to_string()),
            currency: "USD".to_string(),
            credit_limit_minor: Some(1_000_000),
            statement_closing_day: Some(15),
            cc_network: Some("Visa".to_string()),
            metadata: None,
        },
        None,
        "cc-recon-e2e-1".to_string(),
    )
    .await
    .expect("create CC account failed");

    // --- Step 2: Import Chase CC statement ---
    let import_result = import_statement(
        &pool,
        &tenant,
        ImportRequest {
            account_id: cc.id,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 1, // -1500 - 8999 + 20000 - 9500 = 1
            csv_data: chase_cc_csv(),
            filename: Some("chase-feb-2026.csv".to_string()),
            format: Some(CsvFormat::ChaseCredit),
        },
        "cc-recon-corr-1".to_string(),
    )
    .await
    .expect("Chase CC import failed");

    assert_eq!(import_result.lines_imported, 4, "4 Chase CSV lines imported");
    assert_eq!(import_result.lines_skipped, 0);
    assert!(import_result.errors.is_empty());

    // --- Step 3: Ingest CC expense transactions (payment side) ---
    // These are expense records that should match against the CC statement lines.

    // Starbucks: amount matches statement line (-1500), auth=Feb 1, settle=Feb 2
    insert_cc_expense_txn(
        &pool,
        &tenant,
        cc.id,
        -1500,
        NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        "Starbucks purchase",
        None,
        "cc-exp-starbucks-001",
        Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
        Some(NaiveDate::from_ymd_opt(2026, 2, 2).unwrap()),
        Some("STARBUCKS"),
    )
    .await;

    // Amazon: amount matches statement line (-8999), auth=Feb 3, settle=Feb 4
    insert_cc_expense_txn(
        &pool,
        &tenant,
        cc.id,
        -8999,
        NaiveDate::from_ymd_opt(2026, 2, 3).unwrap(),
        "Amazon purchase",
        None,
        "cc-exp-amazon-001",
        Some(NaiveDate::from_ymd_opt(2026, 2, 3).unwrap()),
        Some(NaiveDate::from_ymd_opt(2026, 2, 4).unwrap()),
        Some("AMAZON.COM"),
    )
    .await;

    // Software: -$50.00 — no matching statement line (different amount)
    insert_cc_expense_txn(
        &pool,
        &tenant,
        cc.id,
        -5000,
        NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
        "Software subscription",
        None,
        "cc-exp-software-001",
        Some(NaiveDate::from_ymd_opt(2026, 2, 15).unwrap()),
        Some(NaiveDate::from_ymd_opt(2026, 2, 15).unwrap()),
        Some("SOFTWARECO"),
    )
    .await;

    // Verify total transaction count: 4 statement-sourced + 3 expense-sourced = 7
    let txn_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("txn count query");
    assert_eq!(txn_count, 7, "4 statement lines + 3 expense txns");

    // --- Step 4: Run CC auto-match ---
    // The recon service detects account_type='credit_card' and uses CreditCardStrategy.
    // CC strategy: exact amount + currency required, then date window + merchant scoring.
    //
    // Starbucks (-1500) matches:
    //   - pt.auth_date=Feb1, pt.settle_date=Feb2
    //   - sl.transaction_date=Feb1 in window [Jan31, Feb5]
    //   - days_from_settle = 1 → date_score=0.25
    //   - sl.merchant_name=NULL → merchant_score=0
    //   - confidence = 0.5 + 0.25 = 0.75
    //
    // Amazon (-8999) matches:
    //   - pt.auth_date=Feb3, pt.settle_date=Feb4
    //   - sl.transaction_date=Feb3 in window [Feb2, Feb7]
    //   - days_from_settle = 1 → date_score=0.25
    //   - sl.merchant_name=NULL → merchant_score=0
    //   - confidence = 0.5 + 0.25 = 0.75
    //
    // Software (-5000): no statement line with amount -5000 → no candidate
    // PAYMENT THANK YOU (+20000): no expense txn with +20000 → no candidate
    // ANNUAL FEE (-9500): no expense txn with -9500 → no candidate
    let match_result = run_auto_match(&pool, &tenant, cc.id, "cc-recon-corr-match-1")
        .await
        .expect("CC auto-match failed");

    assert_eq!(match_result.matches_created, 2, "Starbucks + Amazon matched");
    assert_eq!(
        match_result.unmatched_statement_lines, 2,
        "PAYMENT THANK YOU + ANNUAL FEE unmatched"
    );
    assert_eq!(
        match_result.unmatched_transactions, 1,
        "Software expense unmatched"
    );

    // Verify match records in DB
    let matches = list_matches(&pool, &tenant, cc.id, false)
        .await
        .expect("list CC matches failed");
    assert_eq!(matches.len(), 2, "2 active matches");
    for m in &matches {
        assert_eq!(m.match_type, ReconMatchType::Auto, "all auto-matched");
        assert!(m.statement_line_id.is_some(), "each match has statement_line_id");
        assert!(m.superseded_by.is_none(), "no superseded matches");
    }

    // --- Step 5: Verify cash position ---
    // CC account is in credit_card_liability bucket, bank_cash is empty.
    //
    // Transaction total (all 7 txns):
    //   Statement-sourced: -1500 + -8999 + 20000 + -9500 = 1
    //   Expense-sourced:   -1500 + -8999 + -5000 = -15499
    //   Total: 1 + (-15499) = -15498
    //
    // Opening balance from earliest statement = 0
    // CC balance = 0 + (-15498) = -15498
    let pos = get_cash_position(&pool, &tenant)
        .await
        .expect("cash position failed");

    assert!(pos.bank_cash.is_empty(), "no bank accounts — bank_cash empty");
    assert_eq!(pos.credit_card_liability.len(), 1, "1 CC account in liability bucket");

    let cc_pos = &pos.credit_card_liability[0];
    assert_eq!(cc_pos.account_id, cc.id);
    assert_eq!(cc_pos.opening_balance_minor, 0, "opening = 0 from statement");
    // Statement txns: -1500 + -8999 + 20000 + -9500 = 1
    // Expense txns:   -1500 + -8999 + -5000 = -15499
    // Total: -15498
    assert_eq!(cc_pos.transaction_total_minor, -15_498);
    assert_eq!(cc_pos.balance_minor, -15_498, "CC balance is negative (liability)");

    assert_eq!(pos.summary.total_bank_cash_minor, 0);
    assert_eq!(pos.summary.total_cc_liability_minor, -15_498);
    assert_eq!(pos.summary.net_position_minor, -15_498);
    assert_eq!(pos.summary.currencies, vec!["USD"]);

    // --- Step 6: Idempotent rerun ---
    let rerun = run_auto_match(&pool, &tenant, cc.id, "cc-recon-corr-match-2")
        .await
        .expect("CC rerun auto-match failed");

    assert_eq!(rerun.matches_created, 0, "no new matches on rerun");
    assert_eq!(rerun.unmatched_statement_lines, 2, "same 2 unmatched statement lines");
    assert_eq!(rerun.unmatched_transactions, 1, "same 1 unmatched expense");

    // Cash position unchanged
    let pos2 = get_cash_position(&pool, &tenant)
        .await
        .expect("cash position rerun failed");
    assert_eq!(
        pos2.credit_card_liability[0].balance_minor, -15_498,
        "CC balance unchanged after rerun"
    );
    assert_eq!(pos2.summary.net_position_minor, -15_498);

    // Match count unchanged
    let matches2 = list_matches(&pool, &tenant, cc.id, false)
        .await
        .expect("list matches rerun failed");
    assert_eq!(matches2.len(), 2, "still 2 matches after rerun");

    cleanup(&pool, &tenant).await;
}

/// Re-importing the same CC CSV is idempotent (DuplicateImport, no extra rows).
#[tokio::test]
async fn test_cc_reimport_same_statement_idempotent() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    let cc = account_svc::create_credit_card_account(
        &pool,
        &tenant,
        &CreateCreditCardAccountRequest {
            account_name: "Reimport Test CC".to_string(),
            institution: None,
            account_number_last4: None,
            currency: "USD".to_string(),
            credit_limit_minor: Some(500_000),
            statement_closing_day: None,
            cc_network: None,
            metadata: None,
        },
        None,
        "cc-reimport-1".to_string(),
    )
    .await
    .expect("create CC failed");

    let csv = chase_cc_csv();

    // First import
    let r1 = import_statement(
        &pool,
        &tenant,
        ImportRequest {
            account_id: cc.id,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 1,
            csv_data: csv.clone(),
            filename: Some("chase-feb-2026.csv".to_string()),
            format: Some(CsvFormat::ChaseCredit),
        },
        "cc-reimport-corr-1".to_string(),
    )
    .await
    .expect("first CC import failed");
    assert_eq!(r1.lines_imported, 4);

    // Second import with same bytes → DuplicateImport
    let r2 = import_statement(
        &pool,
        &tenant,
        ImportRequest {
            account_id: cc.id,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 1,
            csv_data: csv,
            filename: Some("chase-feb-2026.csv".to_string()),
            format: Some(CsvFormat::ChaseCredit),
        },
        "cc-reimport-corr-2".to_string(),
    )
    .await;

    assert!(
        matches!(r2, Err(treasury::domain::import::ImportError::DuplicateImport { .. })),
        "expected DuplicateImport, got {:?}",
        r2
    );

    // No extra transactions created
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 4, "still only 4 lines from first import");

    cleanup(&pool, &tenant).await;
}

/// Cash position cleanly separates bank cash from CC liability.
///
/// - Bank account: $1000 balance
/// - CC account: -$200 charges
/// - Expected: bank_cash=$1000, cc_liability=-$200, net=$800
#[tokio::test]
async fn test_cc_liability_separated_from_bank_cash() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    // Create bank account
    let bank = account_svc::create_bank_account(
        &pool,
        &tenant,
        &CreateBankAccountRequest {
            account_name: "Operating Checking".to_string(),
            institution: Some("First National".to_string()),
            account_number_last4: Some("1111".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        },
        None,
        "sep-bank-corr-1".to_string(),
    )
    .await
    .expect("create bank account failed");

    // Create CC account
    let cc = account_svc::create_credit_card_account(
        &pool,
        &tenant,
        &CreateCreditCardAccountRequest {
            account_name: "Corp Amex".to_string(),
            institution: Some("American Express".to_string()),
            account_number_last4: Some("9999".to_string()),
            currency: "USD".to_string(),
            credit_limit_minor: Some(500_000),
            statement_closing_day: Some(28),
            cc_network: Some("Amex".to_string()),
            metadata: None,
        },
        None,
        "sep-cc-corr-1".to_string(),
    )
    .await
    .expect("create CC account failed");

    // Insert bank transaction: +$1000 cash received
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, '2026-02-01', 100000, 'USD', 'sep-bank-txn-1')"#,
    )
    .bind(&tenant)
    .bind(bank.id)
    .execute(&pool)
    .await
    .expect("insert bank txn failed");

    // Insert CC charge: -$200.00 (money owed to issuer)
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, '2026-02-05', -20000, 'USD', 'sep-cc-txn-1')"#,
    )
    .bind(&tenant)
    .bind(cc.id)
    .execute(&pool)
    .await
    .expect("insert CC charge failed");

    let pos = get_cash_position(&pool, &tenant)
        .await
        .expect("cash position query failed");

    // Bank cash bucket
    assert_eq!(pos.bank_cash.len(), 1, "1 bank account");
    assert_eq!(pos.bank_cash[0].account_id, bank.id);
    assert_eq!(pos.bank_cash[0].opening_balance_minor, 0);
    assert_eq!(pos.bank_cash[0].transaction_total_minor, 100_000);
    assert_eq!(pos.bank_cash[0].balance_minor, 100_000);

    // CC liability bucket
    assert_eq!(pos.credit_card_liability.len(), 1, "1 CC account in liability bucket");
    assert_eq!(pos.credit_card_liability[0].account_id, cc.id);
    assert_eq!(pos.credit_card_liability[0].opening_balance_minor, 0);
    assert_eq!(pos.credit_card_liability[0].transaction_total_minor, -20_000);
    assert_eq!(pos.credit_card_liability[0].balance_minor, -20_000);

    // Summary
    assert_eq!(pos.summary.total_bank_cash_minor, 100_000);
    assert_eq!(pos.summary.total_cc_liability_minor, -20_000);
    assert_eq!(pos.summary.net_position_minor, 80_000, "net = bank - cc_owed");
    assert_eq!(pos.summary.currencies, vec!["USD"]);

    cleanup(&pool, &tenant).await;
}
