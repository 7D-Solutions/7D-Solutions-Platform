//! Consolidated Statements E2E (Phase 32, bd-1ph5)
//!
//! Proves the full consolidation → statement pipeline against a real
//! PostgreSQL database:
//! 1. Create a consolidation group with two entities
//! 2. Populate consolidated TB cache (simulating close + consolidate)
//! 3. Verify P&L returns correct revenue/expense/net-income
//! 4. Verify Balance Sheet returns correct assets/liabilities/equity
//! 5. Verify eliminations are reflected in statement totals
//! 6. Verify deterministic rerun produces identical results
//!
//! No mocks, no stubs — all tests run against real consolidation
//! PostgreSQL (port 5446).

mod common;

use chrono::NaiveDate;
use common::wait_for_db_ready;
use consolidation::domain::statements::{bs, pl};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test infrastructure
// ============================================================================

fn consolidation_db_url() -> String {
    std::env::var("CONSOLIDATION_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://consolidation_user:consolidation_pass@localhost:5446/consolidation_db"
            .to_string()
    })
}

async fn consolidation_pool() -> PgPool {
    wait_for_db_ready("consolidation", &consolidation_db_url()).await
}

const MIGRATION_LOCK_KEY: i64 = 7_446_319_825_i64;

async fn ensure_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory lock failed");

    let migrations = [
        include_str!(
            "../../modules/consolidation/db/migrations/20260218100001_create_consolidation_config.sql"
        ),
        include_str!(
            "../../modules/consolidation/db/migrations/20260218100002_create_consolidation_caches.sql"
        ),
        include_str!(
            "../../modules/consolidation/db/migrations/20260218100003_create_elimination_postings.sql"
        ),
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

/// Create a test consolidation group and return its ID.
async fn create_test_group(pool: &PgPool, tenant_id: &str) -> Uuid {
    let group_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO csl_groups (id, tenant_id, name, reporting_currency, fiscal_year_end_month)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(group_id)
    .bind(tenant_id)
    .bind("E2E Test Group")
    .bind("USD")
    .bind(12_i16)
    .execute(pool)
    .await
    .expect("create test group");
    group_id
}

/// Insert a row into the consolidated trial balance cache.
async fn insert_tb_row(
    pool: &PgPool,
    group_id: Uuid,
    as_of: &str,
    account_code: &str,
    account_name: &str,
    currency: &str,
    debit: i64,
    credit: i64,
    input_hash: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO csl_trial_balance_cache
            (group_id, as_of, account_code, account_name, currency,
             debit_minor, credit_minor, net_minor, input_hash)
        VALUES ($1, $2::date, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (group_id, as_of, account_code, currency) DO UPDATE SET
            debit_minor  = EXCLUDED.debit_minor,
            credit_minor = EXCLUDED.credit_minor,
            net_minor    = EXCLUDED.net_minor,
            input_hash   = EXCLUDED.input_hash
        "#,
    )
    .bind(group_id)
    .bind(as_of)
    .bind(account_code)
    .bind(account_name)
    .bind(currency)
    .bind(debit)
    .bind(credit)
    .bind(debit - credit)
    .bind(input_hash)
    .execute(pool)
    .await
    .expect("insert consolidated TB row");
}

/// Clean up all test data for a group.
async fn cleanup(pool: &PgPool, group_id: Uuid) {
    sqlx::query("DELETE FROM csl_trial_balance_cache WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_statement_cache WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_elimination_postings WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_coa_mappings WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_fx_policies WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_elimination_rules WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_group_entities WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM csl_groups WHERE id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: Two-entity consolidation → P&L + Balance Sheet with eliminations
// ============================================================================

/// Simulates a two-entity consolidation where:
///   Entity A (parent):  Cash 1000, AR 1100, Revenue 4000, Expenses 6000
///   Entity B (sub):     Cash 1000, AP 2000, Revenue 4000, COGS 5000
///   Elimination:        IC Receivable (1200) / IC Payable (2100) eliminated
///
/// After consolidation + elimination, the consolidated TB shows:
/// - AR 1200 (IC receivable): debit reduced by elimination
/// - AP 2100 (IC payable): credit reduced by elimination
/// Both end up at zero net, proving eliminations are applied.
#[tokio::test]
async fn test_consolidated_statements_two_entity_with_eliminations() {
    let pool = consolidation_pool().await;
    ensure_migrations(&pool).await;
    let tenant_id = format!("e2e-csl-stmt-{}", Uuid::new_v4());
    let group_id = create_test_group(&pool, &tenant_id).await;
    cleanup(&pool, group_id).await;
    // Re-create group after cleanup
    sqlx::query(
        "INSERT INTO csl_groups (id, tenant_id, name, reporting_currency, fiscal_year_end_month)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(group_id)
    .bind(&tenant_id)
    .bind("E2E Test Group")
    .bind("USD")
    .bind(12_i16)
    .execute(&pool)
    .await
    .expect("re-create group");

    let as_of = "2026-01-31";
    let hash = "e2e-test-hash-001";

    // --- Consolidated TB rows (post-elimination) ---
    //
    // Balance sheet accounts:
    //   1000 Cash: Entity A $15,000 DR + Entity B $8,000 DR = $23,000 total
    //   1100 AR (external): Entity A $5,000 DR
    //   1200 IC Receivable: $10,000 DR → eliminated to $0
    //   2000 AP (external): Entity B $3,000 CR
    //   2100 IC Payable: $10,000 CR → eliminated to $0
    //   3000 Equity: Entity A $10,000 CR + Entity B $5,000 CR = $15,000
    //
    // P&L accounts:
    //   4000 Revenue: Entity A $12,000 CR + Entity B $8,000 CR = $20,000
    //   5000 COGS: Entity B $4,000 DR
    //   6000 Expenses: Entity A $3,000 DR + Entity B $1,000 DR = $4,000

    // Assets
    insert_tb_row(&pool, group_id, as_of, "1000", "Cash", "USD", 2_300_000, 0, hash).await;
    insert_tb_row(
        &pool, group_id, as_of, "1100", "Accounts Receivable", "USD", 500_000, 0, hash,
    )
    .await;
    // IC Receivable: fully eliminated (DR 1M, CR 1M from elimination → net 0)
    insert_tb_row(
        &pool,
        group_id,
        as_of,
        "1200",
        "IC Receivable",
        "USD",
        1_000_000,
        1_000_000,
        hash,
    )
    .await;

    // Liabilities
    insert_tb_row(
        &pool, group_id, as_of, "2000", "Accounts Payable", "USD", 0, 300_000, hash,
    )
    .await;
    // IC Payable: fully eliminated (DR 1M from elimination, CR 1M → net 0)
    insert_tb_row(
        &pool,
        group_id,
        as_of,
        "2100",
        "IC Payable",
        "USD",
        1_000_000,
        1_000_000,
        hash,
    )
    .await;

    // Equity
    insert_tb_row(
        &pool, group_id, as_of, "3000", "Retained Earnings", "USD", 0, 1_500_000, hash,
    )
    .await;

    // Revenue
    insert_tb_row(
        &pool, group_id, as_of, "4000", "Revenue", "USD", 0, 2_000_000, hash,
    )
    .await;

    // COGS
    insert_tb_row(&pool, group_id, as_of, "5000", "COGS", "USD", 400_000, 0, hash).await;

    // Expenses
    insert_tb_row(
        &pool,
        group_id,
        as_of,
        "6000",
        "Operating Expenses",
        "USD",
        400_000,
        0,
        hash,
    )
    .await;

    let as_of_date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

    // --- P&L verification ---
    let pl_stmt = pl::compute_consolidated_pl(&pool, group_id, as_of_date)
        .await
        .expect("P&L failed");

    assert_eq!(pl_stmt.group_id, group_id);
    assert_eq!(pl_stmt.as_of, as_of_date);

    let rev = pl_stmt
        .sections
        .iter()
        .find(|s| s.section == "revenue")
        .unwrap();
    let rev_usd = rev.total_by_currency.get("USD").copied().unwrap_or(0);
    assert_eq!(rev_usd, 2_000_000, "Revenue = $20,000.00");

    let cogs = pl_stmt
        .sections
        .iter()
        .find(|s| s.section == "cogs")
        .unwrap();
    let cogs_usd = cogs.total_by_currency.get("USD").copied().unwrap_or(0);
    assert_eq!(cogs_usd, 400_000, "COGS = $4,000.00");

    let exp = pl_stmt
        .sections
        .iter()
        .find(|s| s.section == "expenses")
        .unwrap();
    let exp_usd = exp.total_by_currency.get("USD").copied().unwrap_or(0);
    assert_eq!(exp_usd, 400_000, "Expenses = $4,000.00");

    // Net income = 2,000,000 - 400,000 - 400,000 = 1,200,000
    let net = pl_stmt
        .net_income_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    assert_eq!(net, 1_200_000, "Net income = $12,000.00");

    println!(
        "P&L: revenue={}, cogs={}, expenses={}, net_income={}",
        rev_usd, cogs_usd, exp_usd, net
    );

    // --- Balance Sheet verification ---
    let bs_stmt = bs::compute_consolidated_bs(&pool, group_id, as_of_date)
        .await
        .expect("BS failed");

    assert_eq!(bs_stmt.group_id, group_id);
    assert_eq!(bs_stmt.as_of, as_of_date);

    let assets = bs_stmt
        .sections
        .iter()
        .find(|s| s.section == "assets")
        .unwrap();
    let assets_usd = assets.total_by_currency.get("USD").copied().unwrap_or(0);
    // Cash 2,300,000 + AR 500,000 + IC Recv (1M-1M=0) = 2,800,000
    assert_eq!(assets_usd, 2_800_000, "Total assets = $28,000.00");

    // Verify IC Receivable is net zero
    let ic_recv = assets
        .accounts
        .iter()
        .find(|a| a.account_code == "1200")
        .unwrap();
    assert_eq!(
        ic_recv.amount_minor, 0,
        "IC Receivable eliminated to zero"
    );

    let liabilities = bs_stmt
        .sections
        .iter()
        .find(|s| s.section == "liabilities")
        .unwrap();
    let liab_usd = liabilities
        .total_by_currency
        .get("USD")
        .copied()
        .unwrap_or(0);
    // AP 300,000 + IC Payable (1M-1M=0) = 300,000
    assert_eq!(liab_usd, 300_000, "Total liabilities = $3,000.00");

    // Verify IC Payable is net zero
    let ic_pay = liabilities
        .accounts
        .iter()
        .find(|a| a.account_code == "2100")
        .unwrap();
    assert_eq!(ic_pay.amount_minor, 0, "IC Payable eliminated to zero");

    let equity = bs_stmt
        .sections
        .iter()
        .find(|s| s.section == "equity")
        .unwrap();
    let equity_usd = equity.total_by_currency.get("USD").copied().unwrap_or(0);
    assert_eq!(equity_usd, 1_500_000, "Total equity = $15,000.00");

    println!(
        "BS: assets={}, liabilities={}, equity={}",
        assets_usd, liab_usd, equity_usd
    );

    println!("\nPASS: Two-entity consolidated P&L + BS with eliminations verified");
    cleanup(&pool, group_id).await;
}

// ============================================================================
// Test 2: Deterministic rerun produces identical results
// ============================================================================

#[tokio::test]
async fn test_consolidated_statements_deterministic() {
    let pool = consolidation_pool().await;
    ensure_migrations(&pool).await;
    let tenant_id = format!("e2e-csl-det-{}", Uuid::new_v4());
    let group_id = create_test_group(&pool, &tenant_id).await;
    cleanup(&pool, group_id).await;
    sqlx::query(
        "INSERT INTO csl_groups (id, tenant_id, name, reporting_currency, fiscal_year_end_month)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(group_id)
    .bind(&tenant_id)
    .bind("Determinism Group")
    .bind("USD")
    .bind(12_i16)
    .execute(&pool)
    .await
    .unwrap();

    let as_of = "2026-02-28";
    let hash = "det-hash-001";

    insert_tb_row(&pool, group_id, as_of, "1000", "Cash", "USD", 500_000, 0, hash).await;
    insert_tb_row(&pool, group_id, as_of, "2000", "AP", "USD", 0, 200_000, hash).await;
    insert_tb_row(
        &pool, group_id, as_of, "3000", "Equity", "USD", 0, 300_000, hash,
    )
    .await;
    insert_tb_row(
        &pool, group_id, as_of, "4000", "Revenue", "USD", 0, 800_000, hash,
    )
    .await;
    insert_tb_row(&pool, group_id, as_of, "6000", "Rent", "USD", 150_000, 0, hash).await;

    let date = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

    // Run twice
    let pl1 = pl::compute_consolidated_pl(&pool, group_id, date)
        .await
        .unwrap();
    let pl2 = pl::compute_consolidated_pl(&pool, group_id, date)
        .await
        .unwrap();

    let bs1 = bs::compute_consolidated_bs(&pool, group_id, date)
        .await
        .unwrap();
    let bs2 = bs::compute_consolidated_bs(&pool, group_id, date)
        .await
        .unwrap();

    // P&L determinism
    assert_eq!(pl1.net_income_by_currency, pl2.net_income_by_currency);
    for (s1, s2) in pl1.sections.iter().zip(pl2.sections.iter()) {
        assert_eq!(s1.section, s2.section);
        assert_eq!(s1.total_by_currency, s2.total_by_currency);
        assert_eq!(s1.accounts.len(), s2.accounts.len());
    }

    // BS determinism
    for (s1, s2) in bs1.sections.iter().zip(bs2.sections.iter()) {
        assert_eq!(s1.section, s2.section);
        assert_eq!(s1.total_by_currency, s2.total_by_currency);
        assert_eq!(s1.accounts.len(), s2.accounts.len());
    }

    println!("PASS: Consolidated statements are deterministic");
    cleanup(&pool, group_id).await;
}

// ============================================================================
// Test 3: Empty group returns empty sections (no panic)
// ============================================================================

#[tokio::test]
async fn test_consolidated_statements_empty_group() {
    let pool = consolidation_pool().await;
    ensure_migrations(&pool).await;
    let tenant_id = format!("e2e-csl-empty-{}", Uuid::new_v4());
    let group_id = create_test_group(&pool, &tenant_id).await;
    cleanup(&pool, group_id).await;
    sqlx::query(
        "INSERT INTO csl_groups (id, tenant_id, name, reporting_currency, fiscal_year_end_month)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(group_id)
    .bind(&tenant_id)
    .bind("Empty Group")
    .bind("USD")
    .bind(12_i16)
    .execute(&pool)
    .await
    .unwrap();

    let date = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();

    let pl_stmt = pl::compute_consolidated_pl(&pool, group_id, date)
        .await
        .expect("empty P&L should not error");
    assert!(
        pl_stmt.net_income_by_currency.is_empty(),
        "no income for empty group"
    );
    for s in &pl_stmt.sections {
        assert!(s.accounts.is_empty(), "no accounts in section {}", s.section);
    }

    let bs_stmt = bs::compute_consolidated_bs(&pool, group_id, date)
        .await
        .expect("empty BS should not error");
    for s in &bs_stmt.sections {
        assert!(s.accounts.is_empty(), "no accounts in section {}", s.section);
    }

    println!("PASS: Empty group returns empty statements");
    cleanup(&pool, group_id).await;
}
