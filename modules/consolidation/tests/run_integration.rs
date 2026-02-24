//! Integration tests for consolidation run (cache) + reports (bd-2fdr).
//!
//! Covers:
//! 1. Seed cache and retrieve via get_cached_tb
//! 2. Re-run idempotency — second run replaces first (DELETE + INSERT)
//! 3. get_cached_tb returns None when no cache exists
//! 4. Consolidated P&L from cache (4xxx revenue, 5xxx COGS, 6xxx expenses)
//! 5. Consolidated balance sheet from cache (1xxx assets, 2xxx liab, 3xxx equity)
//! 6. Tenant isolation — different groups have separate caches

use chrono::NaiveDate;
use consolidation::domain::config::{service, CreateGroupRequest};
use consolidation::domain::engine::compute;
use consolidation::domain::statements::{bs, pl};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://consolidation_user:consolidation_pass@localhost:5446/consolidation_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to consolidation test DB");

    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'csl_groups')",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(false);

    if !table_exists {
        sqlx::migrate!("db/migrations")
            .run(&pool)
            .await
            .expect("Failed to run consolidation migrations");
    }

    pool
}

fn unique_tenant() -> String {
    format!("csl-run-{}", Uuid::new_v4().simple())
}

fn group_req(name: &str) -> CreateGroupRequest {
    CreateGroupRequest {
        name: name.to_string(),
        description: None,
        reporting_currency: "USD".to_string(),
        fiscal_year_end_month: Some(12),
    }
}

/// Seed a single row into the trial balance cache (simulating a consolidation run output).
async fn seed_cache_row(
    pool: &sqlx::PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
    account_code: &str,
    account_name: &str,
    currency: &str,
    debit: i64,
    credit: i64,
    input_hash: &str,
) {
    let net = debit - credit;
    sqlx::query(
        "INSERT INTO csl_trial_balance_cache
            (group_id, as_of, account_code, account_name, currency,
             debit_minor, credit_minor, net_minor, input_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (group_id, as_of, account_code, currency) DO UPDATE SET
            debit_minor = EXCLUDED.debit_minor,
            credit_minor = EXCLUDED.credit_minor,
            net_minor = EXCLUDED.net_minor,
            input_hash = EXCLUDED.input_hash,
            computed_at = NOW()",
    )
    .bind(group_id)
    .bind(as_of)
    .bind(account_code)
    .bind(account_name)
    .bind(currency)
    .bind(debit)
    .bind(credit)
    .bind(net)
    .bind(input_hash)
    .execute(pool)
    .await
    .unwrap();
}

/// Delete all cache rows for a group + as_of (simulates clean re-run).
async fn clear_cache(pool: &sqlx::PgPool, group_id: Uuid, as_of: NaiveDate) {
    sqlx::query(
        "DELETE FROM csl_trial_balance_cache WHERE group_id = $1 AND as_of = $2",
    )
    .bind(group_id)
    .bind(as_of)
    .execute(pool)
    .await
    .unwrap();
}

// ============================================================================
// 1. Seed cache and retrieve via get_cached_tb
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cache_seed_and_retrieve() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

    let group = service::create_group(&pool, &tid, &group_req("Cache Group"))
        .await
        .unwrap();

    seed_cache_row(&pool, group.id, as_of, "1000", "Cash", "USD", 100_000, 0, "hash-v1").await;

    let rows = compute::get_cached_tb(&pool, group.id, as_of)
        .await
        .unwrap()
        .expect("expected cached rows");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].account_code, "1000");
    assert_eq!(rows[0].account_name, "Cash");
    assert_eq!(rows[0].debit_minor, 100_000);
    assert_eq!(rows[0].credit_minor, 0);
    assert_eq!(rows[0].net_minor, 100_000);
    assert_eq!(rows[0].input_hash, "hash-v1");
}

// ============================================================================
// 2. Re-run idempotency — second run replaces first (DELETE + INSERT)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cache_idempotent_rerun() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

    let group = service::create_group(&pool, &tid, &group_req("Idem Run Group"))
        .await
        .unwrap();

    // First run — hash-v1
    seed_cache_row(&pool, group.id, as_of, "1000", "Cash", "USD", 100_000, 0, "hash-v1").await;
    let rows = compute::get_cached_tb(&pool, group.id, as_of).await.unwrap().unwrap();
    assert_eq!(rows[0].input_hash, "hash-v1");

    // Second run — delete-then-insert with hash-v2 (idempotent pipeline)
    clear_cache(&pool, group.id, as_of).await;
    seed_cache_row(&pool, group.id, as_of, "1000", "Cash", "USD", 120_000, 0, "hash-v2").await;

    let rows = compute::get_cached_tb(&pool, group.id, as_of).await.unwrap().unwrap();
    assert_eq!(rows.len(), 1, "re-run must not duplicate rows");
    assert_eq!(rows[0].input_hash, "hash-v2", "second run must replace first");
    assert_eq!(rows[0].debit_minor, 120_000, "second run values must be current");
}

// ============================================================================
// 3. get_cached_tb returns None when no cache exists
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_cached_tb_none_when_empty() {
    let pool = setup_db().await;
    let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

    // Random group UUID — no cache exists
    let result = compute::get_cached_tb(&pool, Uuid::new_v4(), as_of)
        .await
        .unwrap();
    assert!(result.is_none(), "no cache should return None");
}

// ============================================================================
// 4. Consolidated P&L from cache
// ============================================================================

#[tokio::test]
#[serial]
async fn test_consolidated_pl_from_cache() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

    let group = service::create_group(&pool, &tid, &group_req("PL Test Group"))
        .await
        .unwrap();

    // Revenue (4xxx, credit-normal): credit_minor - debit_minor
    seed_cache_row(&pool, group.id, as_of, "4000", "Revenue", "USD", 0, 500_000, "hash-pl").await;
    // COGS (5xxx, debit-normal): debit_minor - credit_minor
    seed_cache_row(&pool, group.id, as_of, "5000", "COGS", "USD", 200_000, 0, "hash-pl").await;
    // Expenses (6xxx, debit-normal): debit_minor - credit_minor
    seed_cache_row(&pool, group.id, as_of, "6000", "Salaries", "USD", 100_000, 0, "hash-pl").await;
    // Balance-sheet account (1xxx) — should be excluded from P&L
    seed_cache_row(&pool, group.id, as_of, "1000", "Cash", "USD", 300_000, 0, "hash-pl").await;

    let pl = pl::compute_consolidated_pl(&pool, group.id, as_of)
        .await
        .unwrap();

    assert_eq!(pl.group_id, group.id);
    assert_eq!(pl.as_of, as_of);

    // Revenue section
    let rev = pl.sections.iter().find(|s| s.section == "revenue").unwrap();
    assert_eq!(rev.accounts.len(), 1);
    assert_eq!(rev.accounts[0].account_code, "4000");
    assert_eq!(rev.accounts[0].amount_minor, 500_000); // credit - debit

    // COGS section
    let cogs = pl.sections.iter().find(|s| s.section == "cogs").unwrap();
    assert_eq!(cogs.accounts.len(), 1);
    assert_eq!(cogs.accounts[0].amount_minor, 200_000); // debit - credit

    // Expenses section
    let exp = pl.sections.iter().find(|s| s.section == "expenses").unwrap();
    assert_eq!(exp.accounts.len(), 1);
    assert_eq!(exp.accounts[0].amount_minor, 100_000);

    // Net income: revenue - cogs - expenses = 500k - 200k - 100k = 200k
    let net = pl.net_income_by_currency.get("USD").copied().unwrap_or(0);
    assert_eq!(net, 200_000);
}

// ============================================================================
// 5. Consolidated balance sheet from cache
// ============================================================================

#[tokio::test]
#[serial]
async fn test_consolidated_bs_from_cache() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

    let group = service::create_group(&pool, &tid, &group_req("BS Test Group"))
        .await
        .unwrap();

    // Assets (1xxx, debit-normal)
    seed_cache_row(&pool, group.id, as_of, "1000", "Cash", "USD", 300_000, 0, "hash-bs").await;
    seed_cache_row(&pool, group.id, as_of, "1200", "AR", "USD", 150_000, 0, "hash-bs").await;
    // Liabilities (2xxx, credit-normal)
    seed_cache_row(&pool, group.id, as_of, "2000", "AP", "USD", 0, 100_000, "hash-bs").await;
    // Equity (3xxx, credit-normal)
    seed_cache_row(&pool, group.id, as_of, "3000", "Retained Earnings", "USD", 0, 350_000, "hash-bs").await;
    // P&L account (4xxx) — excluded from BS
    seed_cache_row(&pool, group.id, as_of, "4000", "Revenue", "USD", 0, 500_000, "hash-bs").await;

    let bs = bs::compute_consolidated_bs(&pool, group.id, as_of)
        .await
        .unwrap();

    assert_eq!(bs.group_id, group.id);

    // Assets
    let assets = bs.sections.iter().find(|s| s.section == "assets").unwrap();
    assert_eq!(assets.accounts.len(), 2);
    let total_assets = assets.total_by_currency.get("USD").copied().unwrap_or(0);
    assert_eq!(total_assets, 450_000); // 300k + 150k

    // Liabilities
    let liab = bs.sections.iter().find(|s| s.section == "liabilities").unwrap();
    assert_eq!(liab.accounts.len(), 1);
    assert_eq!(liab.accounts[0].amount_minor, 100_000);

    // Equity
    let eq = bs.sections.iter().find(|s| s.section == "equity").unwrap();
    assert_eq!(eq.accounts.len(), 1);
    assert_eq!(eq.accounts[0].amount_minor, 350_000);
}

// ============================================================================
// 6. Tenant isolation — different groups have separate caches
// ============================================================================

#[tokio::test]
#[serial]
async fn test_run_tenant_isolation_separate_groups() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

    let group_a = service::create_group(&pool, &tid_a, &group_req("Iso Group A"))
        .await
        .unwrap();
    let group_b = service::create_group(&pool, &tid_b, &group_req("Iso Group B"))
        .await
        .unwrap();

    // Only seed group_a's cache
    seed_cache_row(&pool, group_a.id, as_of, "1000", "Cash", "USD", 100_000, 0, "hash-a").await;

    // group_a has cache
    let rows_a = compute::get_cached_tb(&pool, group_a.id, as_of)
        .await
        .unwrap();
    assert!(rows_a.is_some());

    // group_b has no cache — caches are group-scoped
    let rows_b = compute::get_cached_tb(&pool, group_b.id, as_of)
        .await
        .unwrap();
    assert!(rows_b.is_none(), "group B must not see group A's cache");
}
