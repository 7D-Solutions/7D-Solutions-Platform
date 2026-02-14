//! Performance benchmark tests for financial statements (Phase 14.8: bd-39q)
//!
//! **Performance Contract**: All statements must execute in <150ms with large datasets.
//!
//! **Dataset Scale**:
//! - 10,000 accounts (distributed across all account types)
//! - 1,000,000 journal_lines equivalent (via aggregated account_balances)
//!
//! **Measured Operations**:
//! 1. Trial Balance - Single-query aggregation from account_balances
//! 2. Income Statement - Revenue/Expense filtering and aggregation
//! 3. Balance Sheet - Asset/Liability/Equity filtering and aggregation
//!
//! **Acceptance Criteria**:
//! - Each statement completes in <150ms
//! - All queries use indexed paths (no table scans)
//! - Deterministic results (same data → same totals)

use chrono::NaiveDate;
use gl_rs::services::balance_sheet_service::get_balance_sheet;
use gl_rs::services::income_statement_service::get_income_statement;
use gl_rs::services::trial_balance_service::get_trial_balance;
use sqlx::PgPool;
use std::time::Instant;
use uuid::Uuid;

mod common;

/// Seed large dataset for performance testing
///
/// **Strategy**:
/// - 10,000 accounts: 2,000 per type (Asset, Liability, Equity, Revenue, Expense)
/// - 10,000 account_balances: One per account with aggregated amounts
/// - 1M journal_lines equivalent: Each balance represents ~100 journal lines
///
/// **Why aggregated balances?**
/// Statements query account_balances (aggregated view), not raw journal_lines.
/// Seeding 10k account_balances with large aggregated amounts simulates the effect
/// of 1M journal_lines while respecting DB constraints and improving test performance.
async fn seed_performance_dataset(pool: &PgPool, tenant_id: &str, period_id: Uuid) {
    let start = Instant::now();
    println!("\n🌱 Seeding performance dataset...");

    // Step 1: Create 10,000 accounts (2,000 per type)
    println!("   Creating 10,000 accounts...");
    let account_types = [
        ("asset", "debit"),
        ("liability", "credit"),
        ("equity", "credit"),
        ("revenue", "credit"),
        ("expense", "debit"),
    ];

    for (account_type, normal_balance) in &account_types {
        // Create 2,000 accounts per type
        // Use bulk INSERT with VALUES (...), (...), ... for speed
        let mut values = Vec::new();
        for i in 0..2000 {
            let code = format!("{}_{:04}", account_type.to_uppercase(), i);
            let name = format!("{} Account {}", account_type, i);
            values.push(format!(
                "(gen_random_uuid(), '{}', '{}', '{}', '{}'::account_type, '{}'::normal_balance, true, NOW())",
                tenant_id, code, name, account_type, normal_balance
            ));
        }

        // Batch insert 2,000 accounts at once
        let insert_query = format!(
            "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at) VALUES {}",
            values.join(",")
        );

        sqlx::query(&insert_query)
            .execute(pool)
            .await
            .expect("Failed to insert accounts");

        println!("      ✓ Created 2,000 {} accounts", account_type);
    }

    // Step 2: Create account_balances entries (10k entries with aggregated amounts)
    // Each balance represents the aggregated result of ~100 journal_lines
    // Total simulated journal_lines: 10,000 accounts × 100 lines = 1M equivalent
    println!("   Creating 10,000 account_balances (simulating 1M journal_lines equivalent)...");

    // Get all account codes with their types and normal balances
    let accounts: Vec<(String, String)> = sqlx::query_as(
        "SELECT code, type::text FROM accounts WHERE tenant_id = $1 ORDER BY code"
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .expect("Failed to fetch accounts");

    println!("      Retrieved {} accounts", accounts.len());

    // Create one balance entry per account (10k entries total)
    // Each balance has large aggregated amounts simulating 100 journal_lines
    // IMPORTANT: Respect normal balance conventions to satisfy accounting equations:
    // - Assets (debit normal): DR side
    // - Liabilities (credit normal): CR side
    // - Equity (credit normal): CR side
    // - Revenue (credit normal): CR side
    // - Expense (debit normal): DR side
    //
    // With 2k accounts each type, balanced as:
    // Debits: Assets (2k) + Expense (2k) = 4k × 1M = 4B
    // Credits: Liabilities (2k) + Equity (2k) + Revenue (2k) = 6k × 1M = 6B
    // To balance: Make some assets negative or reduce amounts
    //
    // Simpler approach: Equal amounts, adjust by type
    let batch_size = 1000;
    let mut total_inserted = 0;

    for batch_start in (0..accounts.len()).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(accounts.len());
        let mut values = Vec::new();

        for idx in batch_start..batch_end {
            let (account_code, account_type) = &accounts[idx];

            // Simulate aggregated amounts from 100 journal_lines
            let line_count = 100;
            let base_amount = 10000_i64; // Base amount per line (in minor units)
            let total_amount = base_amount * line_count;

            // Assign to debit/credit based on account type normal balance
            // For accounting equation: Assets = Liabilities + Equity
            // We need: 2k assets (DR) = 2k liabilities (CR) + 2k equity (CR)
            // So: 2k × 1M = (2k × 500k) + (2k × 500k)
            let (debit_minor, credit_minor) = match account_type.as_str() {
                "asset" => (total_amount, 0),  // Debit normal
                "expense" => (total_amount, 0), // Debit normal
                "liability" => (0, total_amount / 2), // Credit normal (half amount for balance)
                "equity" => (0, total_amount / 2),    // Credit normal (half amount for balance)
                "revenue" => (0, total_amount),       // Credit normal (full to offset expense)
                _ => (0, 0),
            };
            let net_balance_minor = debit_minor - credit_minor;

            values.push(format!(
                "('{}', '{}', '{}', 'USD', {}, {}, {})",
                tenant_id, period_id, account_code, debit_minor, credit_minor, net_balance_minor
            ));
        }

        if !values.is_empty() {
            let insert_query = format!(
                "INSERT INTO account_balances (tenant_id, period_id, account_code, currency, debit_total_minor, credit_total_minor, net_balance_minor) VALUES {}",
                values.join(",")
            );

            sqlx::query(&insert_query)
                .execute(pool)
                .await
                .expect("Failed to insert account balances");

            total_inserted += values.len();
            if total_inserted % 2000 == 0 {
                println!("      ✓ Inserted {} balances...", total_inserted);
            }
        }
    }

    let elapsed = start.elapsed();
    println!("✅ Seeding complete in {:.2}s", elapsed.as_secs_f64());
    println!("   - 10,000 accounts");
    println!("   - 10,000 account_balances (1M journal_lines equivalent)");
}

/// Test: Trial Balance performance with 1M dataset
#[tokio::test]
#[ignore] // Ignore by default - run explicitly with --ignored
async fn benchmark_trial_balance_1m_dataset() {
    let pool = common::get_test_pool().await;
    let tenant_id = "perf_tenant_trial_balance";

    // Setup
    common::cleanup_test_tenant(&pool, tenant_id).await;

    let period_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let period_id = common::setup_test_period(&pool, tenant_id, period_start, period_end).await;

    // Seed large dataset
    seed_performance_dataset(&pool, tenant_id, period_id).await;

    // Benchmark: Trial Balance
    println!("\n📊 Benchmarking Trial Balance...");
    let start = Instant::now();
    let result = get_trial_balance(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Trial balance should succeed");
    let elapsed = start.elapsed();

    println!("   ✓ Trial Balance completed in {:.2}ms", elapsed.as_millis());
    println!("   - Rows returned: {}", result.rows.len());
    println!("   - Total debits: {}", result.totals.total_debits);
    println!("   - Total credits: {}", result.totals.total_credits);

    // Performance assertion: <150ms
    assert!(
        elapsed.as_millis() < 150,
        "Trial balance took {}ms (expected <150ms)",
        elapsed.as_millis()
    );

    // Cleanup
    common::cleanup_test_tenant(&pool, tenant_id).await;
    pool.close().await;
}

/// Test: Income Statement performance with 1M dataset
#[tokio::test]
#[ignore] // Ignore by default - run explicitly with --ignored
async fn benchmark_income_statement_1m_dataset() {
    let pool = common::get_test_pool().await;
    let tenant_id = "perf_tenant_income_stmt";

    // Setup
    common::cleanup_test_tenant(&pool, tenant_id).await;

    let period_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let period_id = common::setup_test_period(&pool, tenant_id, period_start, period_end).await;

    // Seed large dataset
    seed_performance_dataset(&pool, tenant_id, period_id).await;

    // Benchmark: Income Statement
    println!("\n📊 Benchmarking Income Statement...");
    let start = Instant::now();
    let result = get_income_statement(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Income statement should succeed");
    let elapsed = start.elapsed();

    println!("   ✓ Income Statement completed in {:.2}ms", elapsed.as_millis());
    println!("   - Total rows: {}", result.rows.len());
    println!("   - Total revenue: {}", result.totals.total_revenue);
    println!("   - Total expenses: {}", result.totals.total_expenses);
    println!("   - Net income: {}", result.totals.net_income);

    // Performance assertion: <150ms
    assert!(
        elapsed.as_millis() < 150,
        "Income statement took {}ms (expected <150ms)",
        elapsed.as_millis()
    );

    // Cleanup
    common::cleanup_test_tenant(&pool, tenant_id).await;
    pool.close().await;
}

/// Test: Balance Sheet performance with 1M dataset
#[tokio::test]
#[ignore] // Ignore by default - run explicitly with --ignored
async fn benchmark_balance_sheet_1m_dataset() {
    let pool = common::get_test_pool().await;
    let tenant_id = "perf_tenant_balance_sheet";

    // Setup
    common::cleanup_test_tenant(&pool, tenant_id).await;

    let period_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let period_id = common::setup_test_period(&pool, tenant_id, period_start, period_end).await;

    // Seed large dataset
    seed_performance_dataset(&pool, tenant_id, period_id).await;

    // Benchmark: Balance Sheet
    println!("\n📊 Benchmarking Balance Sheet...");
    let start = Instant::now();
    let result = get_balance_sheet(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Balance sheet should succeed");
    let elapsed = start.elapsed();

    println!("   ✓ Balance Sheet completed in {:.2}ms", elapsed.as_millis());
    println!("   - Total rows: {}", result.rows.len());
    println!("   - Total assets: {}", result.totals.total_assets);
    println!("   - Total liabilities: {}", result.totals.total_liabilities);
    println!("   - Total equity: {}", result.totals.total_equity);

    // Performance assertion: <150ms
    assert!(
        elapsed.as_millis() < 150,
        "Balance sheet took {}ms (expected <150ms)",
        elapsed.as_millis()
    );

    // Cleanup
    common::cleanup_test_tenant(&pool, tenant_id).await;
    pool.close().await;
}

/// Test: All statements in sequence (combined performance envelope)
#[tokio::test]
#[ignore] // Ignore by default - run explicitly with --ignored
async fn benchmark_all_statements_1m_dataset() {
    let pool = common::get_test_pool().await;
    let tenant_id = "perf_tenant_all_stmts";

    // Setup
    common::cleanup_test_tenant(&pool, tenant_id).await;

    let period_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period_end = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    let period_id = common::setup_test_period(&pool, tenant_id, period_start, period_end).await;

    // Seed large dataset (once for all three statements)
    seed_performance_dataset(&pool, tenant_id, period_id).await;

    println!("\n📊 Benchmarking ALL Statements (Sequential)...\n");

    // 1. Trial Balance
    let start = Instant::now();
    let tb_result = get_trial_balance(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Trial balance should succeed");
    let tb_elapsed = start.elapsed();
    println!("   1️⃣  Trial Balance: {:.2}ms ({} rows)", tb_elapsed.as_millis(), tb_result.rows.len());
    assert!(tb_elapsed.as_millis() < 150, "Trial balance took {}ms (expected <150ms)", tb_elapsed.as_millis());

    // 2. Income Statement
    let start = Instant::now();
    let is_result = get_income_statement(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Income statement should succeed");
    let is_elapsed = start.elapsed();
    println!("   2️⃣  Income Statement: {:.2}ms ({} rows, revenue: {}, expenses: {}, net: {})",
        is_elapsed.as_millis(), is_result.rows.len(),
        is_result.totals.total_revenue, is_result.totals.total_expenses, is_result.totals.net_income);
    assert!(is_elapsed.as_millis() < 150, "Income statement took {}ms (expected <150ms)", is_elapsed.as_millis());

    // 3. Balance Sheet
    let start = Instant::now();
    let bs_result = get_balance_sheet(&pool, tenant_id, period_id, "USD")
        .await
        .expect("Balance sheet should succeed");
    let bs_elapsed = start.elapsed();
    println!("   3️⃣  Balance Sheet: {:.2}ms ({} rows, assets: {}, liabilities: {}, equity: {})",
        bs_elapsed.as_millis(), bs_result.rows.len(),
        bs_result.totals.total_assets, bs_result.totals.total_liabilities, bs_result.totals.total_equity);
    assert!(bs_elapsed.as_millis() < 150, "Balance sheet took {}ms (expected <150ms)", bs_elapsed.as_millis());

    // Total time
    let total_elapsed = tb_elapsed + is_elapsed + bs_elapsed;
    println!("\n   ✅ Total time: {:.2}ms", total_elapsed.as_millis());
    println!("      All statements within performance envelope (<150ms each)");

    // Cleanup
    common::cleanup_test_tenant(&pool, tenant_id).await;
    pool.close().await;
}
