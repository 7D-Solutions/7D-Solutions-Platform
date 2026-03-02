//! E2E: Bank statement import + payment ingest + bank recon + cash position (bd-3l8e)
//!
//! Proves the full treasury lifecycle against a real PostgreSQL database:
//! 1. Create a bank account
//! 2. Import a bank statement CSV (creates statement-sourced transactions)
//! 3. Ingest payment events via consumer handler (creates payment-sourced txns)
//! 4. Run auto-match reconciliation
//! 5. Verify match count and cash position balances
//! 6. Rerun auto-match — verify idempotent (0 new matches, same cash position)
//!
//! No mocks, no stubs — all tests run against real treasury PostgreSQL (port 5444).

mod common;

use chrono::NaiveDate;
use common::{generate_test_tenant, wait_for_db_ready};
use sqlx::PgPool;
use uuid::Uuid;

use treasury::consumers::payments::{handle_payment_succeeded, PaymentSucceededPayload};
use treasury::domain::accounts::{service as account_svc, CreateBankAccountRequest};
use treasury::domain::import::service::{import_statement, ImportRequest};
use treasury::domain::import::ImportError;
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

const MIGRATION_LOCK_KEY: i64 = 7_831_294_765_i64;

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
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

/// Deterministic CSV: three transaction lines.
///
/// - 2026-02-01: "Customer Payment Alpha" +$500.00, ref PAY-001
/// - 2026-02-05: "Customer Payment Beta"  +$250.00, ref PAY-002
/// - 2026-02-10: "Interest Income"        +$12.50,  ref INT-001
fn test_csv() -> Vec<u8> {
    b"date,description,amount,reference\n\
      2026-02-01,Customer Payment Alpha,500.00,PAY-001\n\
      2026-02-05,Customer Payment Beta,250.00,PAY-002\n\
      2026-02-10,Interest Income,12.50,INT-001\n"
        .to_vec()
}

async fn create_test_account(
    pool: &PgPool,
    tenant: &str,
) -> treasury::domain::accounts::TreasuryAccount {
    account_svc::create_bank_account(
        pool,
        tenant,
        &CreateBankAccountRequest {
            account_name: "Operating Account".to_string(),
            institution: Some("First National".to_string()),
            account_number_last4: Some("4567".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        },
        None,
        "e2e-recon".to_string(),
    )
    .await
    .expect("create bank account failed")
}

// ============================================================================
// Tests
// ============================================================================

/// Full lifecycle: import → ingest → auto-match → cash position → idempotent rerun.
///
/// Acceptance criteria:
/// - Auto-match produces 2 matches (PAY-001, PAY-002)
/// - 1 unmatched statement line (INT-001), 1 unmatched payment (PAY-003)
/// - Cash position: opening $1000 + txn total $2262.50 = $3262.50
/// - Rerun yields 0 new matches and identical cash position
#[tokio::test]
async fn test_treasury_bank_recon_lifecycle() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    // --- Step 1: Create bank account ---
    let account = create_test_account(&pool, &tenant).await;

    // --- Step 2: Import bank statement ---
    let import_result = import_statement(
        &pool,
        &tenant,
        ImportRequest {
            account_id: account.id,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            opening_balance_minor: 100_000,
            closing_balance_minor: 176_250,
            csv_data: test_csv(),
            filename: Some("feb-2026.csv".to_string()),
            format: None,
        },
        "e2e-corr-1".to_string(),
    )
    .await
    .expect("statement import failed");

    assert_eq!(import_result.lines_imported, 3, "3 CSV lines imported");
    assert_eq!(import_result.lines_skipped, 0);
    assert!(import_result.errors.is_empty());

    // --- Step 3: Ingest payment events ---
    // Payment 1: matches "Customer Payment Alpha" ($500.00, ref PAY-001)
    let inserted1 = handle_payment_succeeded(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &PaymentSucceededPayload {
            payment_id: "PAY-001".to_string(),
            invoice_id: "INV-001".to_string(),
            amount_minor: 50_000,
            currency: "USD".to_string(),
        },
    )
    .await
    .expect("payment 1 ingest failed");
    assert!(inserted1, "payment 1 should insert a bank txn");

    // Payment 2: matches "Customer Payment Beta" ($250.00, ref PAY-002)
    let inserted2 = handle_payment_succeeded(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &PaymentSucceededPayload {
            payment_id: "PAY-002".to_string(),
            invoice_id: "INV-002".to_string(),
            amount_minor: 25_000,
            currency: "USD".to_string(),
        },
    )
    .await
    .expect("payment 2 ingest failed");
    assert!(inserted2, "payment 2 should insert a bank txn");

    // Payment 3: $750 — no matching statement line
    let inserted3 = handle_payment_succeeded(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &PaymentSucceededPayload {
            payment_id: "PAY-003".to_string(),
            invoice_id: "INV-003".to_string(),
            amount_minor: 75_000,
            currency: "USD".to_string(),
        },
    )
    .await
    .expect("payment 3 ingest failed");
    assert!(inserted3, "payment 3 should insert a bank txn");

    // Verify: 6 total transactions (3 statement + 3 payment)
    let txn_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .expect("txn count query");
    assert_eq!(txn_count, 6, "3 statement lines + 3 payment txns");

    // --- Step 4: Run auto-match ---
    let match_result = run_auto_match(&pool, &tenant, account.id, "e2e-corr-match-1")
        .await
        .expect("auto-match failed");

    assert_eq!(
        match_result.matches_created, 2,
        "2 matches: PAY-001 + PAY-002"
    );
    assert_eq!(
        match_result.unmatched_statement_lines, 1,
        "INT-001 unmatched"
    );
    assert_eq!(match_result.unmatched_transactions, 1, "PAY-003 unmatched");

    // Verify matches in DB
    let matches = list_matches(&pool, &tenant, account.id, false)
        .await
        .expect("list matches");
    assert_eq!(matches.len(), 2);
    for m in &matches {
        assert_eq!(m.match_type, ReconMatchType::Auto);
        assert!(m.statement_line_id.is_some());
        assert!(m.superseded_by.is_none());
    }

    // --- Step 5: Verify cash position ---
    let pos = get_cash_position(&pool, &tenant)
        .await
        .expect("cash position failed");

    assert_eq!(pos.bank_cash.len(), 1);
    let bank = &pos.bank_cash[0];
    assert_eq!(bank.account_id, account.id);
    assert_eq!(bank.opening_balance_minor, 100_000);
    // Statement txns: 50000 + 25000 + 1250 = 76250
    // Payment txns:   50000 + 25000 + 75000 = 150000
    // Total: 226250
    assert_eq!(bank.transaction_total_minor, 226_250);
    assert_eq!(bank.balance_minor, 326_250); // 100000 + 226250
    assert_eq!(pos.summary.total_bank_cash_minor, 326_250);
    assert_eq!(pos.summary.total_cc_liability_minor, 0);
    assert_eq!(pos.summary.net_position_minor, 326_250);
    assert_eq!(pos.summary.currencies, vec!["USD"]);

    // --- Step 6: Idempotent rerun ---
    let rerun = run_auto_match(&pool, &tenant, account.id, "e2e-corr-match-2")
        .await
        .expect("rerun auto-match failed");

    assert_eq!(rerun.matches_created, 0, "no new matches on rerun");
    assert_eq!(
        rerun.unmatched_statement_lines, 1,
        "INT-001 still unmatched"
    );
    assert_eq!(rerun.unmatched_transactions, 1, "PAY-003 still unmatched");

    // Cash position unchanged
    let pos2 = get_cash_position(&pool, &tenant)
        .await
        .expect("cash position rerun");
    assert_eq!(pos2.bank_cash[0].balance_minor, 326_250);
    assert_eq!(pos2.summary.net_position_minor, 326_250);

    // Match count unchanged
    let matches2 = list_matches(&pool, &tenant, account.id, false)
        .await
        .expect("list matches rerun");
    assert_eq!(matches2.len(), 2, "same 2 matches after rerun");

    cleanup(&pool, &tenant).await;
}

/// Re-importing the same CSV is idempotent (DuplicateImport, no extra rows).
#[tokio::test]
async fn test_reimport_same_statement_idempotent() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    let account = create_test_account(&pool, &tenant).await;
    let csv = test_csv();

    // First import
    let r1 = import_statement(
        &pool,
        &tenant,
        ImportRequest {
            account_id: account.id,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            opening_balance_minor: 100_000,
            closing_balance_minor: 176_250,
            csv_data: csv.clone(),
            filename: Some("feb-2026.csv".to_string()),
            format: None,
        },
        "e2e-reimport-1".to_string(),
    )
    .await
    .expect("first import");
    assert_eq!(r1.lines_imported, 3);

    // Second import — same CSV bytes → DuplicateImport
    let r2 = import_statement(
        &pool,
        &tenant,
        ImportRequest {
            account_id: account.id,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            opening_balance_minor: 100_000,
            closing_balance_minor: 176_250,
            csv_data: csv,
            filename: Some("feb-2026.csv".to_string()),
            format: None,
        },
        "e2e-reimport-2".to_string(),
    )
    .await;

    assert!(
        matches!(r2, Err(ImportError::DuplicateImport { .. })),
        "expected DuplicateImport, got {:?}",
        r2
    );

    // Verify no extra transactions
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 3, "still only 3 lines from first import");

    cleanup(&pool, &tenant).await;
}

/// Payment event replay is idempotent (same event_id → no duplicate bank txn).
#[tokio::test]
async fn test_payment_ingest_idempotent() {
    let pool = treasury_pool().await;
    ensure_migrations(&pool).await;
    let tenant = generate_test_tenant();
    cleanup(&pool, &tenant).await;

    create_test_account(&pool, &tenant).await;

    let event_id = Uuid::new_v4();
    let payload = PaymentSucceededPayload {
        payment_id: "PAY-IDEM".to_string(),
        invoice_id: "INV-IDEM".to_string(),
        amount_minor: 10_000,
        currency: "USD".to_string(),
    };

    let first = handle_payment_succeeded(&pool, event_id, &tenant, &payload)
        .await
        .expect("first ingest");
    assert!(first, "first ingest should create txn");

    let replay = handle_payment_succeeded(&pool, event_id, &tenant, &payload)
        .await
        .expect("replay");
    assert!(!replay, "replay should be a no-op");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "only 1 txn despite 2 ingest calls");

    cleanup(&pool, &tenant).await;
}
