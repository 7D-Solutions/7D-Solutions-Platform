//! E2E Test: Posting → Balances → Trial Balance (Multi-Currency)
//!
//! This test validates the full path from journal posting to balance updates
//! to trial balance reporting, with multi-currency support and governance enforcement.
//!
//! Test Flow:
//! 1. Set up Chart of Accounts with test accounts
//! 2. Create accounting period
//! 3. Post journal entry in USD
//! 4. Verify balances are updated correctly
//! 5. Verify trial balance reflects correct USD totals
//! 6. Post journal entry in EUR
//! 7. Verify EUR balances are isolated by currency
//! 8. Verify trial balance filtering works correctly

use chrono::{NaiveDate, Utc};
use gl_rs::contracts::gl_posting_request_v1::{Dimensions, JournalLine, GlPostingRequestV1, SourceDocType};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::repos::balance_repo;
use gl_rs::services::journal_service;
use gl_rs::services::trial_balance_service;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Setup test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

/// Helper to insert a test account into Chart of Accounts
async fn insert_test_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: AccountType,
    normal_balance: NormalBalance,
) -> Uuid {
    let id = Uuid::new_v4();

    sqlx::query(
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
    .bind(true) // is_active
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test account");

    id
}

/// Helper to create a test accounting period
async fn insert_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    is_closed: bool,
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
    .bind(is_closed)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test period");

    period_id
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    // Delete in correct order due to foreign key constraints
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup balances");

    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal lines");

    // Get event IDs for this tenant before deleting journal entries
    let event_ids: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT source_event_id FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .expect("Failed to fetch event IDs");

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal entries");

    // Delete processed events by event_id (processed_events doesn't have tenant_id)
    for event_id in event_ids {
        sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
            .bind(event_id)
            .execute(pool)
            .await
            .expect("Failed to cleanup processed event");
    }

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup accounts");

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup periods");
}

#[tokio::test]
#[serial]
async fn test_e2e_posting_updates_balances_and_trial_balance_usd() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-e2e-tb-001";

    // Cleanup any leftover data from previous runs
    cleanup_test_data(&pool, tenant_id).await;

    // Setup: Create Chart of Accounts
    let _acct_ar = insert_test_account(
        &pool,
        tenant_id,
        "1100",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    let _acct_revenue = insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
    )
    .await;

    // Setup: Create accounting period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        false, // open
    )
    .await;

    // Step 1: Post a journal entry in USD
    let event_id = Uuid::new_v4();
    let posting_request = GlPostingRequestV1 {
        posting_date: "2024-02-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv-001".to_string(),
        description: "Test invoice for E2E".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 2599.00,
                credit: 0.0,
                memo: Some("AR debit".to_string()),
                dimensions: Some(Dimensions {
                    customer_id: Some("cust-001".to_string()),
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: None,
                    project: None,
                }),
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 2599.00,
                memo: Some("Revenue credit".to_string()),
                dimensions: None,
            },
        ],
    };

    let entry_id = journal_service::process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "gl.events.posting.requested",
        &posting_request,
    )
    .await
    .expect("Failed to process posting request");

    assert_ne!(entry_id, Uuid::nil(), "Entry ID should be generated");

    // Step 2: Verify balances are updated correctly
    let balance_ar = balance_repo::find_by_grain(&pool, tenant_id, period_id, "1100", "USD")
        .await
        .expect("Failed to query AR balance")
        .expect("AR balance should exist");

    assert_eq!(balance_ar.debit_total_minor, 259900); // $2599.00
    assert_eq!(balance_ar.credit_total_minor, 0);
    assert_eq!(balance_ar.net_balance_minor, 259900);

    let balance_revenue = balance_repo::find_by_grain(&pool, tenant_id, period_id, "4000", "USD")
        .await
        .expect("Failed to query revenue balance")
        .expect("Revenue balance should exist");

    assert_eq!(balance_revenue.debit_total_minor, 0);
    assert_eq!(balance_revenue.credit_total_minor, 259900); // $2599.00
    assert_eq!(balance_revenue.net_balance_minor, -259900);

    // Step 3: Verify trial balance reflects correct USD totals
    let trial_balance = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        Some("USD"),
    )
    .await
    .expect("Failed to get trial balance");

    assert_eq!(trial_balance.tenant_id, tenant_id);
    assert_eq!(trial_balance.period_id, period_id);
    assert_eq!(trial_balance.currency, Some("USD".to_string()));
    assert_eq!(trial_balance.rows.len(), 2, "Should have 2 accounts");

    // Verify totals are balanced
    assert_eq!(trial_balance.totals.total_debits, 259900);
    assert_eq!(trial_balance.totals.total_credits, 259900);
    assert!(trial_balance.totals.is_balanced, "Trial balance should be balanced");

    // Verify individual account rows
    let ar_row = trial_balance.rows.iter().find(|r| r.account_code == "1100").expect("AR account should be in trial balance");
    assert_eq!(ar_row.account_name, "Accounts Receivable");
    assert_eq!(ar_row.account_type, "asset");
    assert_eq!(ar_row.normal_balance, "debit");
    assert_eq!(ar_row.currency, "USD");
    assert_eq!(ar_row.debit_total_minor, 259900);
    assert_eq!(ar_row.credit_total_minor, 0);
    assert_eq!(ar_row.net_balance_minor, 259900);

    let revenue_row = trial_balance.rows.iter().find(|r| r.account_code == "4000").expect("Revenue account should be in trial balance");
    assert_eq!(revenue_row.account_name, "Revenue");
    assert_eq!(revenue_row.account_type, "revenue");
    assert_eq!(revenue_row.normal_balance, "credit");
    assert_eq!(revenue_row.currency, "USD");
    assert_eq!(revenue_row.debit_total_minor, 0);
    assert_eq!(revenue_row.credit_total_minor, 259900);
    assert_eq!(revenue_row.net_balance_minor, -259900);

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    println!("✅ E2E Test Passed: USD posting → balances updated → trial balance correct");
}

#[tokio::test]
#[serial]
async fn test_e2e_multi_currency_isolation_and_filtering() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-e2e-tb-002";

    // Cleanup any leftover data from previous runs
    cleanup_test_data(&pool, tenant_id).await;

    // Setup: Create Chart of Accounts
    let _acct_cash = insert_test_account(
        &pool,
        tenant_id,
        "1000",
        "Cash",
        AccountType::Asset,
        NormalBalance::Debit,
    )
    .await;

    let _acct_equity = insert_test_account(
        &pool,
        tenant_id,
        "3000",
        "Equity",
        AccountType::Equity,
        NormalBalance::Credit,
    )
    .await;

    // Setup: Create accounting period
    let period_id = insert_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        false, // open
    )
    .await;

    // Step 1: Post a journal entry in USD
    let event_usd = Uuid::new_v4();
    let posting_usd = GlPostingRequestV1 {
        posting_date: "2024-03-10".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArAdjustment,
        source_doc_id: "jnl-usd-001".to_string(),
        description: "USD posting".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1000".to_string(),
                debit: 1000.00,
                credit: 0.0,
                memo: None,
                dimensions: None,
            },
            JournalLine {
                account_ref: "3000".to_string(),
                debit: 0.0,
                credit: 1000.00,
                memo: None,
                dimensions: None,
            },
        ],
    };

    journal_service::process_gl_posting_request(
        &pool,
        event_usd,
        tenant_id,
        "gl",
        "gl.events.posting.requested",
        &posting_usd,
    )
    .await
    .expect("Failed to process USD posting");

    // Step 2: Post a journal entry in EUR
    let event_eur = Uuid::new_v4();
    let posting_eur = GlPostingRequestV1 {
        posting_date: "2024-03-15".to_string(),
        currency: "EUR".to_string(),
        source_doc_type: SourceDocType::ArAdjustment,
        source_doc_id: "jnl-eur-001".to_string(),
        description: "EUR posting".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1000".to_string(),
                debit: 500.00,
                credit: 0.0,
                memo: None,
                dimensions: None,
            },
            JournalLine {
                account_ref: "3000".to_string(),
                debit: 0.0,
                credit: 500.00,
                memo: None,
                dimensions: None,
            },
        ],
    };

    journal_service::process_gl_posting_request(
        &pool,
        event_eur,
        tenant_id,
        "gl",
        "gl.events.posting.requested",
        &posting_eur,
    )
    .await
    .expect("Failed to process EUR posting");

    // Step 3: Verify balances are isolated by currency
    let balance_cash_usd = balance_repo::find_by_grain(&pool, tenant_id, period_id, "1000", "USD")
        .await
        .expect("Failed to query USD cash balance")
        .expect("USD cash balance should exist");

    assert_eq!(balance_cash_usd.currency, "USD");
    assert_eq!(balance_cash_usd.debit_total_minor, 100000); // $1000.00
    assert_eq!(balance_cash_usd.net_balance_minor, 100000);

    let balance_cash_eur = balance_repo::find_by_grain(&pool, tenant_id, period_id, "1000", "EUR")
        .await
        .expect("Failed to query EUR cash balance")
        .expect("EUR cash balance should exist");

    assert_eq!(balance_cash_eur.currency, "EUR");
    assert_eq!(balance_cash_eur.debit_total_minor, 50000); // €500.00
    assert_eq!(balance_cash_eur.net_balance_minor, 50000);

    // Step 4: Verify trial balance filtering by currency (USD only)
    let trial_balance_usd = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        Some("USD"),
    )
    .await
    .expect("Failed to get USD trial balance");

    assert_eq!(trial_balance_usd.currency, Some("USD".to_string()));
    assert_eq!(trial_balance_usd.rows.len(), 2, "Should have 2 USD accounts");
    assert_eq!(trial_balance_usd.totals.total_debits, 100000);
    assert_eq!(trial_balance_usd.totals.total_credits, 100000);
    assert!(trial_balance_usd.totals.is_balanced);

    // Verify only USD balances are returned
    for row in &trial_balance_usd.rows {
        assert_eq!(row.currency, "USD", "All rows should be USD");
    }

    // Step 5: Verify trial balance filtering by currency (EUR only)
    let trial_balance_eur = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        Some("EUR"),
    )
    .await
    .expect("Failed to get EUR trial balance");

    assert_eq!(trial_balance_eur.currency, Some("EUR".to_string()));
    assert_eq!(trial_balance_eur.rows.len(), 2, "Should have 2 EUR accounts");
    assert_eq!(trial_balance_eur.totals.total_debits, 50000);
    assert_eq!(trial_balance_eur.totals.total_credits, 50000);
    assert!(trial_balance_eur.totals.is_balanced);

    // Verify only EUR balances are returned
    for row in &trial_balance_eur.rows {
        assert_eq!(row.currency, "EUR", "All rows should be EUR");
    }

    // Step 6: Verify trial balance without currency filter (all currencies)
    let trial_balance_all = trial_balance_service::get_trial_balance(
        &pool,
        tenant_id,
        period_id,
        None,
    )
    .await
    .expect("Failed to get all-currency trial balance");

    assert_eq!(trial_balance_all.currency, None);
    assert_eq!(trial_balance_all.rows.len(), 4, "Should have 4 balances (2 accounts × 2 currencies)");

    // Verify we have both USD and EUR balances
    let usd_rows = trial_balance_all.rows.iter().filter(|r| r.currency == "USD").count();
    let eur_rows = trial_balance_all.rows.iter().filter(|r| r.currency == "EUR").count();
    assert_eq!(usd_rows, 2, "Should have 2 USD rows");
    assert_eq!(eur_rows, 2, "Should have 2 EUR rows");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;

    println!("✅ E2E Test Passed: Multi-currency isolation and filtering works correctly");
}
