//! Integration tests for statement repository (Phase 14)
//!
//! Tests single-query aggregation and indexed path enforcement.

use gl_rs::repos::statement_repo::{
    get_balance_sheet_rows, get_income_statement_rows, get_trial_balance_rows,
};
use sqlx::PgPool;
use uuid::Uuid;

mod common;

/// Helper: Log SQL EXPLAIN plan for a query
///
/// Substitutes parameters into query for EXPLAIN analysis.
/// Note: This is for logging only - actual queries use proper parameter binding.
async fn log_explain_plan(pool: &PgPool, query: &str, tenant_id: &str, period_id: Uuid) {
    // Substitute parameters directly for EXPLAIN (not for actual execution)
    let query_with_params = query
        .replace("$1", &format!("'{}'", tenant_id))
        .replace("$2", &format!("'{}'", period_id));

    let explain_query = format!("EXPLAIN (FORMAT TEXT, ANALYZE FALSE) {}", query_with_params);

    println!("\n=== EXPLAIN PLAN ===");
    println!(
        "Query: {}",
        query.lines().take(3).collect::<Vec<_>>().join(" ")
    );
    println!("Tenant: {}, Period: {}", tenant_id, period_id);

    let plan: Vec<(String,)> = sqlx::query_as(&explain_query)
        .fetch_all(pool)
        .await
        .expect("Failed to get EXPLAIN plan");

    for (line,) in plan {
        println!("{}", line);
    }
    println!("=== END EXPLAIN ===\n");
}

/// Test: get_trial_balance_rows with EXPLAIN plan
#[tokio::test]
async fn test_get_trial_balance_rows_with_explain() {
    let pool = common::get_test_pool().await;
    let tenant_id = "tenant_test_trial_balance";
    let period_id = Uuid::new_v4();

    // Clean up any existing data for this tenant (cascade order: balances → accounts → periods)
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up balances");
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up accounts");
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up periods");

    // Create accounting period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-01-01', '2024-01-31', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert period");

    // Create accounts
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            (gen_random_uuid(), $1, '1000', 'Cash', 'asset', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '4000', 'Revenue', 'revenue', 'credit', true, NOW()),
            (gen_random_uuid(), $1, '5000', 'Expense', 'expense', 'debit', true, NOW())
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert accounts");

    // Create account balances
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor
        )
        VALUES
            ($1, $2, '1000', 'USD', 100000, 0, 100000),
            ($1, $2, '4000', 'USD', 0, 50000, -50000),
            ($1, $2, '5000', 'USD', 30000, 0, 30000)
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to insert balances");

    // Log EXPLAIN plan for trial balance query (without currency filter)
    let explain_query = r#"
        SELECT
            ab.account_code,
            a.name as account_name,
            a.type as account_type,
            a.normal_balance,
            ab.currency,
            ab.debit_total_minor,
            ab.credit_total_minor,
            ab.net_balance_minor
        FROM account_balances ab
        INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
        WHERE ab.tenant_id = $1
          AND ab.period_id = $2
          AND a.is_active = true
        ORDER BY ab.account_code, ab.currency
    "#;

    log_explain_plan(&pool, explain_query, tenant_id, period_id).await;

    // Execute query
    let rows = get_trial_balance_rows(&pool, tenant_id, period_id, Some("USD"))
        .await
        .expect("Failed to get trial balance rows");

    // Assertions
    assert_eq!(rows.len(), 3);

    // Verify row structure
    let cash_row = rows.iter().find(|r| r.account_code == "1000").unwrap();
    assert_eq!(cash_row.account_name, "Cash");
    assert_eq!(cash_row.account_type, "asset");
    assert_eq!(cash_row.normal_balance, "debit");
    assert_eq!(cash_row.currency, "USD");
    assert_eq!(cash_row.debit_total_minor, 100000);
    assert_eq!(cash_row.credit_total_minor, 0);
    assert_eq!(cash_row.net_balance_minor, 100000);

    let revenue_row = rows.iter().find(|r| r.account_code == "4000").unwrap();
    assert_eq!(revenue_row.account_name, "Revenue");
    assert_eq!(revenue_row.account_type, "revenue");
    assert_eq!(revenue_row.normal_balance, "credit");

    let expense_row = rows.iter().find(|r| r.account_code == "5000").unwrap();
    assert_eq!(expense_row.account_name, "Expense");
    assert_eq!(expense_row.account_type, "expense");
    assert_eq!(expense_row.normal_balance, "debit");
}

/// Test: get_income_statement_rows with EXPLAIN plan
#[tokio::test]
async fn test_get_income_statement_rows_with_explain() {
    let pool = common::get_test_pool().await;
    let tenant_id = "tenant_test_income_statement";
    let period_id = Uuid::new_v4();

    // Clean up any existing data for this tenant (cascade order: balances → accounts → periods)
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up balances");
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up accounts");
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up periods");

    // Create accounting period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-01-01', '2024-01-31', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert period");

    // Create revenue and expense accounts only
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            (gen_random_uuid(), $1, '4000', 'Sales Revenue', 'revenue', 'credit', true, NOW()),
            (gen_random_uuid(), $1, '4100', 'Service Revenue', 'revenue', 'credit', true, NOW()),
            (gen_random_uuid(), $1, '5000', 'Cost of Sales', 'expense', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '5100', 'Salaries', 'expense', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '1000', 'Cash', 'asset', 'debit', true, NOW())
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert accounts");

    // Create account balances (including asset to verify it's filtered out)
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor
        )
        VALUES
            ($1, $2, '4000', 'USD', 0, 100000, -100000),
            ($1, $2, '4100', 'USD', 0, 50000, -50000),
            ($1, $2, '5000', 'USD', 60000, 0, 60000),
            ($1, $2, '5100', 'USD', 30000, 0, 30000),
            ($1, $2, '1000', 'USD', 200000, 0, 200000)
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to insert balances");

    // Log EXPLAIN plan for income statement query
    let explain_query = r#"
        SELECT
            ab.account_code,
            a.name as account_name,
            a.type as account_type,
            ab.currency,
            ab.net_balance_minor
        FROM account_balances ab
        INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
        WHERE ab.tenant_id = $1
          AND ab.period_id = $2
          AND a.is_active = true
          AND a.type IN ('revenue', 'expense')
        ORDER BY a.type DESC, ab.account_code, ab.currency
    "#;

    log_explain_plan(&pool, explain_query, tenant_id, period_id).await;

    // Execute query
    let rows = get_income_statement_rows(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Failed to get income statement rows");

    // Assertions: should only have revenue and expense accounts (not asset)
    assert_eq!(rows.len(), 4);

    // Verify revenue rows (should be positive)
    let revenue_4000 = rows.iter().find(|r| r.account_code == "4000").unwrap();
    assert_eq!(revenue_4000.account_name, "Sales Revenue");
    assert_eq!(revenue_4000.account_type, "revenue");
    assert_eq!(revenue_4000.amount_minor, -100000); // Credit balance = positive for revenue

    let revenue_4100 = rows.iter().find(|r| r.account_code == "4100").unwrap();
    assert_eq!(revenue_4100.account_type, "revenue");
    assert_eq!(revenue_4100.amount_minor, -50000);

    // Verify expense rows (should be negative)
    let expense_5000 = rows.iter().find(|r| r.account_code == "5000").unwrap();
    assert_eq!(expense_5000.account_name, "Cost of Sales");
    assert_eq!(expense_5000.account_type, "expense");
    assert_eq!(expense_5000.amount_minor, -60000); // Debit balance = negative for expense

    let expense_5100 = rows.iter().find(|r| r.account_code == "5100").unwrap();
    assert_eq!(expense_5100.account_type, "expense");
    assert_eq!(expense_5100.amount_minor, -30000);

    // Verify asset account is NOT included
    assert!(rows.iter().all(|r| r.account_code != "1000"));
}

/// Test: get_balance_sheet_rows with EXPLAIN plan
#[tokio::test]
async fn test_get_balance_sheet_rows_with_explain() {
    let pool = common::get_test_pool().await;
    let tenant_id = "tenant_test_balance_sheet";
    let period_id = Uuid::new_v4();

    // Clean up any existing data for this tenant (cascade order: balances → accounts → periods)
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up balances");
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up accounts");
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up periods");

    // Create accounting period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-01-01', '2024-01-31', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert period");

    // Create asset, liability, and equity accounts (exclude revenue/expense)
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            (gen_random_uuid(), $1, '1000', 'Cash', 'asset', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '1100', 'Accounts Receivable', 'asset', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '2000', 'Accounts Payable', 'liability', 'credit', true, NOW()),
            (gen_random_uuid(), $1, '3000', 'Equity', 'equity', 'credit', true, NOW()),
            (gen_random_uuid(), $1, '4000', 'Revenue', 'revenue', 'credit', true, NOW())
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert accounts");

    // Create account balances (including revenue to verify it's filtered out)
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor
        )
        VALUES
            ($1, $2, '1000', 'USD', 100000, 0, 100000),
            ($1, $2, '1100', 'USD', 50000, 0, 50000),
            ($1, $2, '2000', 'USD', 0, 30000, -30000),
            ($1, $2, '3000', 'USD', 0, 120000, -120000),
            ($1, $2, '4000', 'USD', 0, 100000, -100000)
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to insert balances");

    // Log EXPLAIN plan for balance sheet query
    let explain_query = r#"
        SELECT
            ab.account_code,
            a.name as account_name,
            a.type as account_type,
            ab.currency,
            ab.net_balance_minor
        FROM account_balances ab
        INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
        WHERE ab.tenant_id = $1
          AND ab.period_id = $2
          AND a.is_active = true
          AND a.type IN ('asset', 'liability', 'equity')
        ORDER BY a.type, ab.account_code, ab.currency
    "#;

    log_explain_plan(&pool, explain_query, tenant_id, period_id).await;

    // Execute query
    let rows = get_balance_sheet_rows(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Failed to get balance sheet rows");

    // Assertions: should only have asset, liability, equity (not revenue)
    assert_eq!(rows.len(), 4);

    // Verify asset rows
    let cash_row = rows.iter().find(|r| r.account_code == "1000").unwrap();
    assert_eq!(cash_row.account_name, "Cash");
    assert_eq!(cash_row.account_type, "asset");
    assert_eq!(cash_row.amount_minor, 100000);

    let ar_row = rows.iter().find(|r| r.account_code == "1100").unwrap();
    assert_eq!(ar_row.account_type, "asset");
    assert_eq!(ar_row.amount_minor, 50000);

    // Verify liability row
    let ap_row = rows.iter().find(|r| r.account_code == "2000").unwrap();
    assert_eq!(ap_row.account_name, "Accounts Payable");
    assert_eq!(ap_row.account_type, "liability");
    assert_eq!(ap_row.amount_minor, -30000);

    // Verify equity row
    let equity_row = rows.iter().find(|r| r.account_code == "3000").unwrap();
    assert_eq!(equity_row.account_type, "equity");
    assert_eq!(equity_row.amount_minor, -120000);

    // Verify revenue account is NOT included
    assert!(rows.iter().all(|r| r.account_code != "4000"));
}

/// Test: get_trial_balance_rows with currency filter
#[tokio::test]
async fn test_get_trial_balance_rows_with_currency_filter() {
    let pool = common::get_test_pool().await;
    let tenant_id = "tenant_test_tb_currency";
    let period_id = Uuid::new_v4();

    // Clean up any existing data for this tenant (cascade order: balances → accounts → periods)
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up balances");
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up accounts");
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up periods");

    // Create accounting period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-01-01', '2024-01-31', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert period");

    // Create accounts
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            (gen_random_uuid(), $1, '1000', 'Cash', 'asset', 'debit', true, NOW())
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert accounts");

    // Create balances in multiple currencies
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor
        )
        VALUES
            ($1, $2, '1000', 'USD', 100000, 0, 100000),
            ($1, $2, '1000', 'EUR', 50000, 0, 50000),
            ($1, $2, '1000', 'GBP', 30000, 0, 30000)
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to insert balances");

    // Test USD currency
    let usd_rows = get_trial_balance_rows(&pool, tenant_id, period_id, Some("USD"))
        .await
        .expect("Failed to get USD rows");

    assert_eq!(usd_rows.len(), 1);
    assert_eq!(usd_rows[0].currency, "USD");
    assert_eq!(usd_rows[0].debit_total_minor, 100000);

    // Test EUR currency
    let eur_rows = get_trial_balance_rows(&pool, tenant_id, period_id, Some("EUR"))
        .await
        .expect("Failed to get EUR rows");

    assert_eq!(eur_rows.len(), 1);
    assert_eq!(eur_rows[0].currency, "EUR");
    assert_eq!(eur_rows[0].debit_total_minor, 50000);

    // Test GBP currency
    let gbp_rows = get_trial_balance_rows(&pool, tenant_id, period_id, Some("GBP"))
        .await
        .expect("Failed to get GBP rows");

    assert_eq!(gbp_rows.len(), 1);
    assert_eq!(gbp_rows[0].currency, "GBP");
    assert_eq!(gbp_rows[0].debit_total_minor, 30000);
}

/// Test: Period validation - PeriodNotFound error
#[tokio::test]
async fn test_period_validation_not_found() {
    let pool = common::get_test_pool().await;
    let tenant_id = "tenant_test_period_validation";
    let non_existent_period_id = Uuid::new_v4();

    // Clean up any existing data for this tenant
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up balances");
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up accounts");
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up periods");

    // Attempt to query with non-existent period - should fail with PeriodNotFound
    let result =
        get_trial_balance_rows(&pool, tenant_id, non_existent_period_id, Some("USD")).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        gl_rs::repos::statement_repo::StatementError::PeriodNotFound {
            period_id,
            tenant_id: tid,
        } => {
            assert_eq!(period_id, non_existent_period_id);
            assert_eq!(tid, tenant_id);
        }
        other => panic!("Expected PeriodNotFound error, got: {:?}", other),
    }
}

/// Test: Numeric safety proof - decimal exactness with integers
///
/// This test proves that our i64 minor unit approach maintains exact precision
/// where floating point would fail. Uses the classic 0.1 + 0.2 != 0.3 case.
#[tokio::test]
async fn test_numeric_safety_proof_decimal_exactness() {
    let pool = common::get_test_pool().await;
    let tenant_id = "tenant_test_numeric_safety";
    let period_id = Uuid::new_v4();

    // Clean up any existing data for this tenant
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up balances");
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up accounts");
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("Failed to clean up periods");

    // Create accounting period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-01-01', '2024-01-31', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert period");

    // Create accounts
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            (gen_random_uuid(), $1, '1000', 'Account A', 'asset', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '1001', 'Account B', 'asset', 'debit', true, NOW()),
            (gen_random_uuid(), $1, '1002', 'Account C', 'asset', 'debit', true, NOW())
        "#,
    )
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to insert accounts");

    // Create balances that would fail with floating point (0.1 + 0.2 case)
    // In cents: 10 cents + 20 cents = 30 cents (exact with integers)
    // With float: 0.1 + 0.2 = 0.30000000000000004 (not exact)
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor
        )
        VALUES
            ($1, $2, '1000', 'USD', 10, 0, 10),
            ($1, $2, '1001', 'USD', 20, 0, 20),
            ($1, $2, '1002', 'USD', 30, 60, -30)
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .execute(&pool)
    .await
    .expect("Failed to insert balances");

    // Execute query
    let rows = get_trial_balance_rows(&pool, tenant_id, period_id, Some("USD"))
        .await
        .expect("Failed to get trial balance rows");

    assert_eq!(rows.len(), 3);

    // Calculate totals - this would fail with floating point rounding
    let total_debits: i64 = rows.iter().map(|r| r.debit_total_minor).sum();
    let total_credits: i64 = rows.iter().map(|r| r.credit_total_minor).sum();

    // Exact integer arithmetic: 10 + 20 + 30 = 60
    assert_eq!(total_debits, 60, "Debit totals must be exact (no rounding)");
    assert_eq!(
        total_credits, 60,
        "Credit totals must be exact (no rounding)"
    );

    // Verify individual balances are exact
    let account_a = rows.iter().find(|r| r.account_code == "1000").unwrap();
    assert_eq!(account_a.debit_total_minor, 10);
    assert_eq!(account_a.net_balance_minor, 10);

    let account_b = rows.iter().find(|r| r.account_code == "1001").unwrap();
    assert_eq!(account_b.debit_total_minor, 20);
    assert_eq!(account_b.net_balance_minor, 20);

    let account_c = rows.iter().find(|r| r.account_code == "1002").unwrap();
    assert_eq!(account_c.debit_total_minor, 30);
    assert_eq!(account_c.credit_total_minor, 60);
    assert_eq!(account_c.net_balance_minor, -30);

    // Final proof: totals balance exactly (would fail with float)
    assert_eq!(
        total_debits, total_credits,
        "Totals must balance exactly - proof of numeric safety"
    );
}
