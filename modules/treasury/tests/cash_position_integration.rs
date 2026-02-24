//! Integrated tests for Treasury cash position and cash forecast (bd-2ztc).
//!
//! Covers:
//! 1. Cash position — empty returns zero summary
//! 2. Cash position — bank and CC in separate buckets
//! 3. Cash position — net position and opening balance
//! 4. Cash position — tenant isolation
//! 5. Forecast — empty inputs produce empty result
//! 6. Forecast — AR inflows with correct rate application

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use treasury::domain::accounts::{
    service as acct_svc, CreateBankAccountRequest, CreateCreditCardAccountRequest,
};
use treasury::domain::reports::{
    assumptions::ForecastAssumptions,
    cash_position,
    forecast::{self, ArAgingInput},
};

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to treasury test DB");

    // Run migrations. If the schema was bootstrapped outside of sqlx (no tracking rows),
    // the first migration will fail with "already exists". Accept that and verify the
    // schema is accessible — the DB is ready either way.
    if let Err(e) = sqlx::migrate!("db/migrations").run(&pool).await {
        if !e.to_string().contains("already exists") {
            panic!("Failed to run treasury migrations: {}", e);
        }
        sqlx::query("SELECT 1 FROM treasury_bank_accounts LIMIT 0")
            .execute(&pool)
            .await
            .expect("treasury_bank_accounts not accessible after migration fallback");
    }

    pool
}

fn unique_app() -> String {
    format!("cp-test-{}", Uuid::new_v4().simple())
}

async fn create_bank_account(pool: &sqlx::PgPool, app: &str, name: &str) -> Uuid {
    acct_svc::create_bank_account(
        pool,
        app,
        &CreateBankAccountRequest {
            account_name: name.to_string(),
            institution: Some("Test Bank".to_string()),
            account_number_last4: Some("1111".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        },
        None,
        format!("setup-{}", Uuid::new_v4()),
    )
    .await
    .expect("create bank account")
    .id
}

async fn create_cc_account(pool: &sqlx::PgPool, app: &str, name: &str) -> Uuid {
    acct_svc::create_credit_card_account(
        pool,
        app,
        &CreateCreditCardAccountRequest {
            account_name: name.to_string(),
            institution: Some("Chase".to_string()),
            account_number_last4: Some("9999".to_string()),
            currency: "USD".to_string(),
            credit_limit_minor: Some(500_000),
            statement_closing_day: Some(15),
            cc_network: Some("Visa".to_string()),
            metadata: None,
        },
        None,
        format!("setup-{}", Uuid::new_v4()),
    )
    .await
    .expect("create CC account")
    .id
}

// ============================================================================
// 1. Cash position — empty returns zero summary
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cash_position_empty_returns_zeros() {
    let pool = setup_db().await;
    let app = unique_app();

    let pos = cash_position::get_cash_position(&pool, &app)
        .await
        .expect("query failed");

    assert!(pos.bank_cash.is_empty());
    assert!(pos.credit_card_liability.is_empty());
    assert_eq!(pos.summary.total_bank_cash_minor, 0);
    assert_eq!(pos.summary.total_cc_liability_minor, 0);
    assert_eq!(pos.summary.net_position_minor, 0);
    assert!(pos.summary.currencies.is_empty());
}

// ============================================================================
// 2. Cash position — bank and CC in separate buckets
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cash_position_bank_and_cc_separated() {
    let pool = setup_db().await;
    let app = unique_app();

    let bank_id = create_bank_account(&pool, &app, "Checking").await;
    let cc_id = create_cc_account(&pool, &app, "Corp Visa").await;

    // Insert transactions for both
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, '2026-01-15', 100000, 'USD', $3)"#,
    )
    .bind(&app)
    .bind(bank_id)
    .bind(format!("bank-{}", Uuid::new_v4()))
    .execute(&pool)
    .await
    .expect("insert bank txn");

    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, '2026-01-20', -30000, 'USD', $3)"#,
    )
    .bind(&app)
    .bind(cc_id)
    .bind(format!("cc-{}", Uuid::new_v4()))
    .execute(&pool)
    .await
    .expect("insert CC txn");

    let pos = cash_position::get_cash_position(&pool, &app)
        .await
        .expect("query failed");

    assert_eq!(pos.bank_cash.len(), 1);
    assert_eq!(pos.bank_cash[0].account_id, bank_id);
    assert_eq!(pos.bank_cash[0].balance_minor, 100_000);

    assert_eq!(pos.credit_card_liability.len(), 1);
    assert_eq!(pos.credit_card_liability[0].account_id, cc_id);
    assert_eq!(pos.credit_card_liability[0].balance_minor, -30_000);

    assert_eq!(pos.summary.total_bank_cash_minor, 100_000);
    assert_eq!(pos.summary.total_cc_liability_minor, -30_000);
    assert_eq!(pos.summary.net_position_minor, 70_000);
}

// ============================================================================
// 3. Cash position — opening balance from statement included in position
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cash_position_includes_opening_balance() {
    let pool = setup_db().await;
    let app = unique_app();

    let bank_id = create_bank_account(&pool, &app, "Savings").await;

    // Insert a statement with opening balance
    sqlx::query(
        r#"INSERT INTO treasury_bank_statements
           (app_id, account_id, period_start, period_end,
            opening_balance_minor, closing_balance_minor, currency, status)
           VALUES ($1, $2, '2026-01-01', '2026-01-31', 500000, 520000, 'USD',
                   'reconciled'::treasury_statement_status)"#,
    )
    .bind(&app)
    .bind(bank_id)
    .execute(&pool)
    .await
    .expect("insert statement");

    // Post-statement transaction
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, '2026-02-05', 25000, 'USD', $3)"#,
    )
    .bind(&app)
    .bind(bank_id)
    .bind(format!("post-stmt-{}", Uuid::new_v4()))
    .execute(&pool)
    .await
    .expect("insert txn");

    let pos = cash_position::get_cash_position(&pool, &app)
        .await
        .expect("query failed");

    assert_eq!(pos.bank_cash.len(), 1);
    // opening (500_000) + txn (25_000) = 525_000
    assert_eq!(pos.bank_cash[0].opening_balance_minor, 500_000);
    assert_eq!(pos.bank_cash[0].transaction_total_minor, 25_000);
    assert_eq!(pos.bank_cash[0].balance_minor, 525_000);
}

// ============================================================================
// 4. Cash position — tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cash_position_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    // Create account under app_a and insert transactions
    let bank_id = create_bank_account(&pool, &app_a, "Isolated").await;
    sqlx::query(
        r#"INSERT INTO treasury_bank_transactions
           (app_id, account_id, transaction_date, amount_minor, currency, external_id)
           VALUES ($1, $2, '2026-01-01', 99999, 'USD', $3)"#,
    )
    .bind(&app_a)
    .bind(bank_id)
    .bind(format!("iso-{}", Uuid::new_v4()))
    .execute(&pool)
    .await
    .expect("insert txn");

    // app_b sees nothing
    let pos_b = cash_position::get_cash_position(&pool, &app_b)
        .await
        .expect("query failed");

    assert!(pos_b.bank_cash.is_empty());
    assert_eq!(pos_b.summary.net_position_minor, 0);
}

// ============================================================================
// 5. Forecast — empty inputs produce empty result
// ============================================================================

#[tokio::test]
#[serial]
async fn test_forecast_empty_inputs() {
    let _pool = setup_db().await; // run migrations; forecast is pure computation
    let assumptions = ForecastAssumptions::default();

    let resp = forecast::compute_forecast(&[], &[], &[], &assumptions, vec![]);

    assert!(resp.forecasts.is_empty());
    assert!(!resp.methodology.is_empty());
    assert_eq!(resp.assumptions.ar_current_rate, 0.95);
}

// ============================================================================
// 6. Forecast — AR inflows with correct rate application
// ============================================================================

#[tokio::test]
#[serial]
async fn test_forecast_ar_inflows_apply_rates() {
    let _pool = setup_db().await; // run migrations
    let assumptions = ForecastAssumptions::default();

    let ar = vec![ArAgingInput {
        currency: "USD".to_string(),
        current_minor: 100_000,
        days_1_30_minor: 50_000,
        days_31_60_minor: 0,
        days_61_90_minor: 0,
        days_over_90_minor: 0,
    }];

    let resp = forecast::compute_forecast(&ar, &[], &[], &assumptions, vec!["ar".into()]);

    assert_eq!(resp.forecasts.len(), 1);
    let f = &resp.forecasts[0];
    assert_eq!(f.currency, "USD");
    // current: 100_000 * 0.95 = 95_000
    assert_eq!(f.inflows.current_minor, 95_000);
    // 1-30: 50_000 * 0.85 = 42_500
    assert_eq!(f.inflows.days_1_30_minor, 42_500);
    assert_eq!(f.inflows.total_minor, 137_500);
    assert_eq!(f.outflows.total_minor, 0);
    assert_eq!(f.total_net_minor, 137_500);
    assert_eq!(resp.data_sources, vec!["ar"]);
}
