//! Balance Sheet computed from the trial balance cache (cumulative as_of date).
//!
//! Account classification by code prefix:
//!   1xxx → Assets      (debit-normal:  amount = debit_minor − credit_minor)
//!   2xxx → Liabilities (credit-normal: amount = credit_minor − debit_minor)
//!   3xxx → Equity      (credit-normal: amount = credit_minor − debit_minor)
//!
//! Balance sheet invariant: total_assets = total_liabilities + total_equity
//! (per currency, assuming all retained earnings have been posted to 3xxx).
//!
//! Date semantics: sums all `rpt_trial_balance_cache` rows where
//! `as_of <= as_of_date` (cumulative balances, not period-specific).

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BsAccountLine {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    /// Minor units. Positive = normal balance for this section's sign convention.
    pub amount_minor: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BsSection {
    pub section: String,
    pub accounts: Vec<BsAccountLine>,
    /// Total per currency across all accounts in this section.
    pub total_by_currency: HashMap<String, i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BalanceSheet {
    pub as_of: NaiveDate,
    /// Ordered: ["assets", "liabilities", "equity"]
    pub sections: Vec<BsSection>,
}

// ── Computation ───────────────────────────────────────────────────────────────

/// Compute a Balance Sheet from the cumulative trial balance as of `as_of`.
pub async fn compute_balance_sheet(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<BalanceSheet, sqlx::Error> {
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_code,
               MAX(account_name)           AS account_name,
               currency,
               SUM(debit_minor)::BIGINT    AS debit_minor,
               SUM(credit_minor)::BIGINT   AS credit_minor
        FROM rpt_trial_balance_cache
        WHERE tenant_id = $1
          AND as_of <= $2
        GROUP BY account_code, currency
        ORDER BY account_code, currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;

    let mut assets: Vec<BsAccountLine> = Vec::new();
    let mut liabilities: Vec<BsAccountLine> = Vec::new();
    let mut equity: Vec<BsAccountLine> = Vec::new();

    for (account_code, account_name, currency, debit_minor, credit_minor) in rows {
        let prefix = account_prefix(&account_code);
        match prefix {
            1 => assets.push(BsAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: debit_minor - credit_minor,
            }),
            2 => liabilities.push(BsAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: credit_minor - debit_minor,
            }),
            3 => equity.push(BsAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: credit_minor - debit_minor,
            }),
            _ => {} // P&L accounts — not part of the balance sheet snapshot
        }
    }

    let sections = vec![
        BsSection {
            section: "assets".into(),
            total_by_currency: sum_by_currency(&assets),
            accounts: assets,
        },
        BsSection {
            section: "liabilities".into(),
            total_by_currency: sum_by_currency(&liabilities),
            accounts: liabilities,
        },
        BsSection {
            section: "equity".into(),
            total_by_currency: sum_by_currency(&equity),
            accounts: equity,
        },
    ];

    Ok(BalanceSheet { as_of, sections })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn account_prefix(code: &str) -> u32 {
    code.chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .unwrap_or(0)
}

fn sum_by_currency(lines: &[BsAccountLine]) -> HashMap<String, i64> {
    let mut m: HashMap<String, i64> = HashMap::new();
    for l in lines {
        *m.entry(l.currency.clone()).or_insert(0) += l.amount_minor;
    }
    m
}

// ── Integrated tests (real DB, no mocks) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TENANT: &str = "test-bs-tenant";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/reporting_test".into())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM rpt_trial_balance_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
    }

    async fn insert_row(
        pool: &PgPool,
        as_of: &str,
        account_code: &str,
        account_name: &str,
        currency: &str,
        debit: i64,
        credit: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO rpt_trial_balance_cache
                (tenant_id, as_of, account_code, account_name, currency,
                 debit_minor, credit_minor, net_minor)
            VALUES ($1, $2::date, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (tenant_id, as_of, account_code, currency) DO UPDATE SET
                debit_minor  = EXCLUDED.debit_minor,
                credit_minor = EXCLUDED.credit_minor,
                net_minor    = EXCLUDED.net_minor
            "#,
        )
        .bind(TENANT)
        .bind(as_of)
        .bind(account_code)
        .bind(account_name)
        .bind(currency)
        .bind(debit)
        .bind(credit)
        .bind(debit - credit)
        .execute(pool)
        .await
        .expect("insert trial balance row");
    }

    // ── Test 1: basic assets, liabilities, equity ────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_bs_basic_sections() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Asset: 1100 AR debit 2599.00 → 259900
        insert_row(&pool, "2026-01-15", "1100", "AR", "USD", 259900, 0).await;
        // Liability: 2000 AP credit 500.00 → 50000
        insert_row(&pool, "2026-01-15", "2000", "AP", "USD", 0, 50000).await;
        // Equity: 3000 Retained Earnings credit 200000
        insert_row(
            &pool,
            "2026-01-15",
            "3000",
            "Retained Earnings",
            "USD",
            0,
            200000,
        )
        .await;

        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");
        let bs = compute_balance_sheet(&pool, TENANT, as_of)
            .await
            .expect("compute_bs");

        let assets = bs.sections.iter().find(|s| s.section == "assets").expect("assets section");
        assert_eq!(
            assets.total_by_currency.get("USD").copied().unwrap_or(0),
            259900
        );

        let liabilities = bs
            .sections
            .iter()
            .find(|s| s.section == "liabilities")
            .expect("liabilities section");
        assert_eq!(
            liabilities
                .total_by_currency
                .get("USD")
                .copied()
                .unwrap_or(0),
            50000
        );

        let equity = bs.sections.iter().find(|s| s.section == "equity").expect("equity section");
        assert_eq!(
            equity.total_by_currency.get("USD").copied().unwrap_or(0),
            200000
        );

        cleanup(&pool).await;
    }

    // ── Test 2: cumulative — includes rows from before as_of ─────────────────

    #[tokio::test]
    #[serial]
    async fn test_bs_cumulative_as_of() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Two postings on different dates
        insert_row(&pool, "2026-01-10", "1000", "Cash", "USD", 100000, 0).await;
        insert_row(&pool, "2026-02-05", "1000", "Cash", "USD", 50000, 0).await;
        // Posting after as_of — must be excluded
        insert_row(&pool, "2026-03-01", "1000", "Cash", "USD", 999999, 0).await;

        let as_of = NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date");
        let bs = compute_balance_sheet(&pool, TENANT, as_of)
            .await
            .expect("compute_bs");

        let assets = bs.sections.iter().find(|s| s.section == "assets").expect("assets section");
        assert_eq!(
            assets.total_by_currency.get("USD").copied().unwrap_or(0),
            150000,
            "should sum Jan + Feb but not March posting"
        );

        cleanup(&pool).await;
    }

    // ── Test 3: P&L accounts excluded from balance sheet ─────────────────────

    #[tokio::test]
    #[serial]
    async fn test_bs_excludes_pl_accounts() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        insert_row(&pool, "2026-02-01", "4000", "Revenue", "USD", 0, 100000).await;
        insert_row(&pool, "2026-02-01", "5000", "COGS", "USD", 40000, 0).await;
        insert_row(&pool, "2026-02-01", "1000", "Cash", "USD", 60000, 0).await;

        let as_of = NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date");
        let bs = compute_balance_sheet(&pool, TENANT, as_of)
            .await
            .expect("compute_bs");

        let assets = bs.sections.iter().find(|s| s.section == "assets").expect("assets section");
        assert_eq!(assets.accounts.len(), 1, "only 1xxx account in assets");

        let liabilities = bs
            .sections
            .iter()
            .find(|s| s.section == "liabilities")
            .expect("liabilities section");
        assert!(liabilities.accounts.is_empty());

        cleanup(&pool).await;
    }

    // ── Test 4: multi-currency isolation ─────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_bs_multi_currency() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        insert_row(&pool, "2026-02-01", "1000", "Cash", "USD", 100000, 0).await;
        insert_row(&pool, "2026-02-01", "1000", "Cash", "EUR", 80000, 0).await;

        let as_of = NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date");
        let bs = compute_balance_sheet(&pool, TENANT, as_of)
            .await
            .expect("compute_bs");

        let assets = bs.sections.iter().find(|s| s.section == "assets").expect("assets section");
        assert_eq!(
            assets.total_by_currency.get("USD").copied().unwrap_or(0),
            100000
        );
        assert_eq!(
            assets.total_by_currency.get("EUR").copied().unwrap_or(0),
            80000
        );

        cleanup(&pool).await;
    }
}
