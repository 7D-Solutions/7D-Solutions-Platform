//! Profit & Loss statement computed from the trial balance cache.
//!
//! Account classification by code prefix:
//!   4xxx → Revenue  (credit-normal: amount = credit_minor − debit_minor)
//!   5xxx → COGS     (debit-normal:  amount = debit_minor − credit_minor)
//!   6xxx → Expenses (debit-normal:  amount = debit_minor − credit_minor)
//!
//! Date range semantics: sums all `rpt_trial_balance_cache` rows where
//! `as_of BETWEEN from AND to` (both inclusive).

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PlAccountLine {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    /// Signed minor units. Positive = revenue/expense amount as labelled.
    pub amount_minor: i64,
}

#[derive(Debug, Serialize)]
pub struct PlSection {
    pub section: String,
    pub accounts: Vec<PlAccountLine>,
    /// Total per currency across all accounts in this section.
    pub total_by_currency: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct PlStatement {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Ordered: ["revenue", "cogs", "expenses"]
    pub sections: Vec<PlSection>,
    /// Net income (revenue − cogs − expenses) per currency.
    pub net_income_by_currency: HashMap<String, i64>,
}

// ── Computation ───────────────────────────────────────────────────────────────

/// Compute a P&L statement from the trial balance cache for the given period.
pub async fn compute_pl(
    pool: &PgPool,
    tenant_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<PlStatement, sqlx::Error> {
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_code,
               MAX(account_name)           AS account_name,
               currency,
               SUM(debit_minor)::BIGINT    AS debit_minor,
               SUM(credit_minor)::BIGINT   AS credit_minor
        FROM rpt_trial_balance_cache
        WHERE tenant_id = $1
          AND as_of BETWEEN $2 AND $3
        GROUP BY account_code, currency
        ORDER BY account_code, currency
        "#,
    )
    .bind(tenant_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let mut revenue: Vec<PlAccountLine> = Vec::new();
    let mut cogs: Vec<PlAccountLine> = Vec::new();
    let mut expenses: Vec<PlAccountLine> = Vec::new();

    for (account_code, account_name, currency, debit_minor, credit_minor) in rows {
        let prefix = account_prefix(&account_code);
        match prefix {
            4 => revenue.push(PlAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: credit_minor - debit_minor,
            }),
            5 => cogs.push(PlAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: debit_minor - credit_minor,
            }),
            6 => expenses.push(PlAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: debit_minor - credit_minor,
            }),
            _ => {} // balance-sheet accounts — not part of P&L
        }
    }

    let rev_totals = sum_by_currency(&revenue);
    let cogs_totals = sum_by_currency(&cogs);
    let exp_totals = sum_by_currency(&expenses);

    // Net income per currency: revenue − cogs − expenses
    let mut net: HashMap<String, i64> = HashMap::new();
    for (cur, &rev) in &rev_totals {
        *net.entry(cur.clone()).or_insert(0) += rev;
    }
    for (cur, &cost) in &cogs_totals {
        *net.entry(cur.clone()).or_insert(0) -= cost;
    }
    for (cur, &exp) in &exp_totals {
        *net.entry(cur.clone()).or_insert(0) -= exp;
    }

    let sections = vec![
        PlSection {
            section: "revenue".into(),
            total_by_currency: rev_totals,
            accounts: revenue,
        },
        PlSection {
            section: "cogs".into(),
            total_by_currency: cogs_totals,
            accounts: cogs,
        },
        PlSection {
            section: "expenses".into(),
            total_by_currency: exp_totals,
            accounts: expenses,
        },
    ];

    Ok(PlStatement {
        from,
        to,
        sections,
        net_income_by_currency: net,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn account_prefix(code: &str) -> u32 {
    code.chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .unwrap_or(0)
}

fn sum_by_currency(lines: &[PlAccountLine]) -> HashMap<String, i64> {
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

    const TENANT: &str = "test-pl-tenant";

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

    // ── Test 1: basic P&L with revenue and expense ───────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_pl_basic_revenue_and_expense() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Revenue: 4000 credit 2599.00 → amount = 259900
        insert_row(&pool, "2026-01-15", "4000", "Revenue", "USD", 0, 259900).await;
        // Expense: 5000 debit 500.00 → amount = 50000
        insert_row(&pool, "2026-01-20", "5000", "COGS", "USD", 50000, 0).await;

        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let stmt = compute_pl(&pool, TENANT, from, to)
            .await
            .expect("compute_pl failed");

        let rev = stmt
            .sections
            .iter()
            .find(|s| s.section == "revenue")
            .unwrap();
        assert_eq!(
            rev.total_by_currency.get("USD").copied().unwrap_or(0),
            259900
        );

        let cogs = stmt.sections.iter().find(|s| s.section == "cogs").unwrap();
        assert_eq!(
            cogs.total_by_currency.get("USD").copied().unwrap_or(0),
            50000
        );

        // net income = 259900 − 50000 = 209900
        assert_eq!(
            stmt.net_income_by_currency.get("USD").copied().unwrap_or(0),
            209900
        );

        cleanup(&pool).await;
    }

    // ── Test 2: date range filter excludes out-of-range rows ─────────────────

    #[tokio::test]
    #[serial]
    async fn test_pl_date_range_filter() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // In range
        insert_row(&pool, "2026-02-10", "4000", "Revenue", "USD", 0, 100000).await;
        // Out of range (before from)
        insert_row(&pool, "2026-01-31", "4000", "Revenue", "USD", 0, 999999).await;
        // Out of range (after to)
        insert_row(&pool, "2026-03-01", "4000", "Revenue", "USD", 0, 888888).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_pl(&pool, TENANT, from, to)
            .await
            .expect("compute_pl");

        let rev = stmt
            .sections
            .iter()
            .find(|s| s.section == "revenue")
            .unwrap();
        assert_eq!(
            rev.total_by_currency.get("USD").copied().unwrap_or(0),
            100000,
            "only in-range rows should be included"
        );

        cleanup(&pool).await;
    }

    // ── Test 3: balance-sheet accounts (1xxx, 2xxx, 3xxx) are excluded ───────

    #[tokio::test]
    #[serial]
    async fn test_pl_ignores_balance_sheet_accounts() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        insert_row(&pool, "2026-02-15", "1100", "AR", "USD", 50000, 0).await;
        insert_row(&pool, "2026-02-15", "2000", "AP", "USD", 0, 50000).await;
        insert_row(&pool, "2026-02-15", "3000", "Equity", "USD", 0, 50000).await;
        insert_row(&pool, "2026-02-15", "4000", "Revenue", "USD", 0, 10000).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_pl(&pool, TENANT, from, to)
            .await
            .expect("compute_pl");

        let rev = stmt
            .sections
            .iter()
            .find(|s| s.section == "revenue")
            .unwrap();
        assert_eq!(rev.accounts.len(), 1, "only 4xxx in revenue section");
        assert_eq!(
            rev.total_by_currency.get("USD").copied().unwrap_or(0),
            10000
        );

        let cogs = stmt.sections.iter().find(|s| s.section == "cogs").unwrap();
        assert!(cogs.accounts.is_empty(), "no 5xxx accounts posted");

        cleanup(&pool).await;
    }

    // ── Test 4: multi-currency grouping ──────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_pl_multi_currency() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        insert_row(&pool, "2026-02-01", "4000", "Revenue", "USD", 0, 100000).await;
        insert_row(&pool, "2026-02-01", "4000", "Revenue", "EUR", 0, 80000).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_pl(&pool, TENANT, from, to)
            .await
            .expect("compute_pl");

        let rev = stmt
            .sections
            .iter()
            .find(|s| s.section == "revenue")
            .unwrap();
        assert_eq!(
            rev.total_by_currency.get("USD").copied().unwrap_or(0),
            100000
        );
        assert_eq!(
            rev.total_by_currency.get("EUR").copied().unwrap_or(0),
            80000
        );
        assert_eq!(
            stmt.net_income_by_currency.get("USD").copied().unwrap_or(0),
            100000
        );
        assert_eq!(
            stmt.net_income_by_currency.get("EUR").copied().unwrap_or(0),
            80000
        );

        cleanup(&pool).await;
    }
}
