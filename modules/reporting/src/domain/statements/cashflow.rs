//! Cash flow statement (indirect method v1) from GL + Payments events.
//!
//! ## Data sources
//!
//! - **Payments events** → `rpt_cashflow_cache` rows with `line_code = 'cash_collections'`
//!   (accumulated by [`crate::ingest::payments::PaymentsHandler`]).
//! - **GL trial balance** → `rpt_trial_balance_cache` for period net income.
//!
//! ## Classification
//!
//! Operating activities:
//!   - `net_income`:       Revenue (4xxx) − COGS (5xxx) − Expenses (6xxx) from GL
//!   - `cash_collections`: Direct cash from Payments events
//!
//! Investing / Financing: empty for v1 (no event sources yet).
//!
//! ## Caching
//!
//! Computed GL-derived lines are written back to `rpt_cashflow_cache` so repeat
//! queries for the same `(from, to)` window hit the cache.
//!
//! ## Known limitations
//!
//! - Investing and financing sections are stubs (always zero).
//! - Working-capital adjustments (AR/AP changes) are not modelled in v1.
//! - True indirect-method reconciliation requires future enhancements.

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CashflowLine {
    pub line_code: String,
    pub line_label: String,
    pub currency: String,
    /// Signed minor units. Positive = cash inflow.
    pub amount_minor: i64,
}

#[derive(Debug, Serialize)]
pub struct CashflowSection {
    pub activity_type: String,
    pub lines: Vec<CashflowLine>,
    /// Total per currency across all lines in this section.
    pub total_by_currency: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct CashflowStatement {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Ordered: ["operating", "investing", "financing"]
    pub sections: Vec<CashflowSection>,
    /// Net cash change per currency (sum of all sections).
    pub net_cash_change_by_currency: HashMap<String, i64>,
}

// ── Computation ──────────────────────────────────────────────────────────────

/// Compute a cash flow statement for the given period.
///
/// 1. Check if GL-derived lines are already cached for this exact window.
/// 2. If not, compute net income from trial balance and cache it.
/// 3. Aggregate payment collection lines from the cashflow cache.
/// 4. Build operating / investing / financing sections.
pub async fn compute_cashflow(
    pool: &PgPool,
    tenant_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<CashflowStatement, sqlx::Error> {
    // Step 1: ensure GL-derived net_income is cached for this window
    ensure_gl_lines_cached(pool, tenant_id, from, to).await?;

    // Step 2: read all cashflow cache lines for the window
    let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        r#"
        SELECT activity_type, line_code, line_label, currency,
               SUM(amount_minor)::BIGINT AS amount_minor
        FROM rpt_cashflow_cache
        WHERE tenant_id = $1
          AND period_start >= $2
          AND period_end   <= $3
        GROUP BY activity_type, line_code, line_label, currency
        ORDER BY activity_type, line_code, currency
        "#,
    )
    .bind(tenant_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    // Step 3: bucket into sections
    let mut operating_lines: Vec<CashflowLine> = Vec::new();
    let mut investing_lines: Vec<CashflowLine> = Vec::new();
    let mut financing_lines: Vec<CashflowLine> = Vec::new();

    for (activity_type, line_code, line_label, currency, amount_minor) in rows {
        let line = CashflowLine { line_code, line_label, currency, amount_minor };
        match activity_type.as_str() {
            "operating" => operating_lines.push(line),
            "investing" => investing_lines.push(line),
            "financing" => financing_lines.push(line),
            _ => {} // ignore unknown
        }
    }

    let operating_totals = sum_by_currency(&operating_lines);
    let investing_totals = sum_by_currency(&investing_lines);
    let financing_totals = sum_by_currency(&financing_lines);

    // Net cash change = operating + investing + financing per currency
    let mut net: HashMap<String, i64> = HashMap::new();
    for map in [&operating_totals, &investing_totals, &financing_totals] {
        for (cur, &amt) in map {
            *net.entry(cur.clone()).or_insert(0) += amt;
        }
    }

    let sections = vec![
        CashflowSection {
            activity_type: "operating".into(),
            total_by_currency: operating_totals,
            lines: operating_lines,
        },
        CashflowSection {
            activity_type: "investing".into(),
            total_by_currency: investing_totals,
            lines: investing_lines,
        },
        CashflowSection {
            activity_type: "financing".into(),
            total_by_currency: financing_totals,
            lines: financing_lines,
        },
    ];

    Ok(CashflowStatement { from, to, sections, net_cash_change_by_currency: net })
}

// ── GL-derived cache builder ─────────────────────────────────────────────────

/// Compute net income from the trial balance cache and write it to the
/// cashflow cache if not already present for this window.
async fn ensure_gl_lines_cached(
    pool: &PgPool,
    tenant_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(), sqlx::Error> {
    // Check if net_income is already cached for this exact window
    let exists: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT 1::BIGINT FROM rpt_cashflow_cache
        WHERE tenant_id    = $1
          AND period_start = $2
          AND period_end   = $3
          AND line_code    = 'net_income'
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(from)
    .bind(to)
    .fetch_optional(pool)
    .await?;

    if exists.is_some() {
        return Ok(());
    }

    // Compute net income per currency from trial balance: revenue − COGS − expenses
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_code, currency,
               SUM(debit_minor)::BIGINT  AS debit_minor,
               SUM(credit_minor)::BIGINT AS credit_minor
        FROM rpt_trial_balance_cache
        WHERE tenant_id = $1
          AND as_of BETWEEN $2 AND $3
        GROUP BY account_code, currency
        "#,
    )
    .bind(tenant_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let mut net_income: HashMap<String, i64> = HashMap::new();
    for (account_code, currency, debit_minor, credit_minor) in &rows {
        let prefix = account_code.chars().next().and_then(|c| c.to_digit(10)).unwrap_or(0);
        let amount = match prefix {
            4 => credit_minor - debit_minor, // Revenue (credit-normal)
            5 => -(debit_minor - credit_minor), // COGS (subtract from income)
            6 => -(debit_minor - credit_minor), // Expenses (subtract from income)
            _ => continue,
        };
        *net_income.entry(currency.clone()).or_insert(0) += amount;
    }

    // Cache net_income lines
    for (currency, amount) in &net_income {
        sqlx::query(
            r#"
            INSERT INTO rpt_cashflow_cache
                (tenant_id, period_start, period_end, activity_type,
                 line_code, line_label, currency, amount_minor, computed_at)
            VALUES ($1, $2, $3, 'operating', 'net_income',
                    'Net income (from GL)', $4, $5, NOW())
            ON CONFLICT (tenant_id, period_start, period_end,
                         activity_type, line_code, currency)
            DO UPDATE SET
                amount_minor = EXCLUDED.amount_minor,
                computed_at  = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(from)
        .bind(to)
        .bind(currency)
        .bind(amount)
        .execute(pool)
        .await?;
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn sum_by_currency(lines: &[CashflowLine]) -> HashMap<String, i64> {
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

    const TENANT: &str = "test-cashflow-stmt-tenant";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/reporting_test".into())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        sqlx::migrate!("./db/migrations").run(&pool).await.expect("migrate");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM rpt_cashflow_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM rpt_trial_balance_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
    }

    async fn insert_cashflow_line(
        pool: &PgPool,
        period_start: &str,
        period_end: &str,
        activity_type: &str,
        line_code: &str,
        line_label: &str,
        currency: &str,
        amount: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO rpt_cashflow_cache
                (tenant_id, period_start, period_end, activity_type,
                 line_code, line_label, currency, amount_minor)
            VALUES ($1, $2::date, $3::date, $4, $5, $6, $7, $8)
            ON CONFLICT (tenant_id, period_start, period_end,
                         activity_type, line_code, currency)
            DO UPDATE SET amount_minor = EXCLUDED.amount_minor
            "#,
        )
        .bind(TENANT)
        .bind(period_start)
        .bind(period_end)
        .bind(activity_type)
        .bind(line_code)
        .bind(line_label)
        .bind(currency)
        .bind(amount)
        .execute(pool)
        .await
        .expect("insert cashflow line");
    }

    async fn insert_trial_balance(
        pool: &PgPool,
        as_of: &str,
        account_code: &str,
        currency: &str,
        debit: i64,
        credit: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO rpt_trial_balance_cache
                (tenant_id, as_of, account_code, account_name, currency,
                 debit_minor, credit_minor, net_minor)
            VALUES ($1, $2::date, $3, $3, $4, $5, $6, $7)
            ON CONFLICT (tenant_id, as_of, account_code, currency)
            DO UPDATE SET debit_minor = EXCLUDED.debit_minor,
                          credit_minor = EXCLUDED.credit_minor,
                          net_minor = EXCLUDED.net_minor
            "#,
        )
        .bind(TENANT)
        .bind(as_of)
        .bind(account_code)
        .bind(currency)
        .bind(debit)
        .bind(credit)
        .bind(debit - credit)
        .execute(pool)
        .await
        .expect("insert trial balance");
    }

    // ── Test 1: cash collections from Payments ───────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_cashflow_collections_only() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Simulate ingested payment collections (daily)
        insert_cashflow_line(
            &pool, "2026-02-10", "2026-02-10",
            "operating", "cash_collections", "Customer collections", "USD", 150000,
        ).await;
        insert_cashflow_line(
            &pool, "2026-02-15", "2026-02-15",
            "operating", "cash_collections", "Customer collections", "USD", 100000,
        ).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_cashflow(&pool, TENANT, from, to).await.expect("compute");

        let operating = stmt.sections.iter().find(|s| s.activity_type == "operating").unwrap();
        let usd_total = operating.total_by_currency.get("USD").copied().unwrap_or(0);

        // 150000 + 100000 = 250000
        assert_eq!(usd_total, 250000);

        // Collections line present
        let coll: Vec<&CashflowLine> = operating.lines.iter()
            .filter(|l| l.line_code == "cash_collections" && l.currency == "USD")
            .collect();
        assert!(!coll.is_empty());

        cleanup(&pool).await;
    }

    // ── Test 2: GL-derived net income is computed and cached ─────────────────

    #[tokio::test]
    #[serial]
    async fn test_cashflow_gl_net_income() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Revenue: 4000 credit 3000.00
        insert_trial_balance(&pool, "2026-02-15", "4000", "USD", 0, 300000).await;
        // COGS: 5000 debit 1000.00
        insert_trial_balance(&pool, "2026-02-15", "5000", "USD", 100000, 0).await;
        // Expense: 6000 debit 500.00
        insert_trial_balance(&pool, "2026-02-15", "6000", "USD", 50000, 0).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_cashflow(&pool, TENANT, from, to).await.expect("compute");

        let operating = stmt.sections.iter().find(|s| s.activity_type == "operating").unwrap();

        // Net income = 300000 - 100000 - 50000 = 150000
        let ni: Vec<&CashflowLine> = operating.lines.iter()
            .filter(|l| l.line_code == "net_income" && l.currency == "USD")
            .collect();
        assert_eq!(ni.len(), 1);
        assert_eq!(ni[0].amount_minor, 150000);

        // Verify cached (second call should hit cache)
        let stmt2 = compute_cashflow(&pool, TENANT, from, to).await.expect("compute2");
        let op2 = stmt2.sections.iter().find(|s| s.activity_type == "operating").unwrap();
        let ni2: Vec<&CashflowLine> = op2.lines.iter()
            .filter(|l| l.line_code == "net_income" && l.currency == "USD")
            .collect();
        assert_eq!(ni2[0].amount_minor, 150000, "cached value must match");

        cleanup(&pool).await;
    }

    // ── Test 3: combined GL + Payments ───────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_cashflow_combined_gl_and_payments() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // GL net income data
        insert_trial_balance(&pool, "2026-02-10", "4000", "USD", 0, 200000).await;
        insert_trial_balance(&pool, "2026-02-10", "5000", "USD", 50000, 0).await;
        // net_income = 200000 - 50000 = 150000

        // Payments collections (daily ingested)
        insert_cashflow_line(
            &pool, "2026-02-12", "2026-02-12",
            "operating", "cash_collections", "Customer collections", "USD", 180000,
        ).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_cashflow(&pool, TENANT, from, to).await.expect("compute");

        let operating = stmt.sections.iter().find(|s| s.activity_type == "operating").unwrap();
        // Operating total = net_income(150000) + cash_collections(180000) = 330000
        assert_eq!(operating.total_by_currency.get("USD").copied().unwrap_or(0), 330000);

        // Net cash change = same (investing + financing are zero)
        assert_eq!(stmt.net_cash_change_by_currency.get("USD").copied().unwrap_or(0), 330000);

        cleanup(&pool).await;
    }

    // ── Test 4: multi-currency isolation ─────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_cashflow_multi_currency() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        insert_cashflow_line(
            &pool, "2026-02-10", "2026-02-10",
            "operating", "cash_collections", "Customer collections", "USD", 100000,
        ).await;
        insert_cashflow_line(
            &pool, "2026-02-10", "2026-02-10",
            "operating", "cash_collections", "Customer collections", "EUR", 80000,
        ).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_cashflow(&pool, TENANT, from, to).await.expect("compute");

        let operating = stmt.sections.iter().find(|s| s.activity_type == "operating").unwrap();
        assert_eq!(operating.total_by_currency.get("USD").copied().unwrap_or(0), 100000);
        assert_eq!(operating.total_by_currency.get("EUR").copied().unwrap_or(0), 80000);

        cleanup(&pool).await;
    }

    // ── Test 5: date range filter ────────────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_cashflow_date_range_filter() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // In range
        insert_cashflow_line(
            &pool, "2026-02-10", "2026-02-10",
            "operating", "cash_collections", "Customer collections", "USD", 100000,
        ).await;
        // Out of range (before)
        insert_cashflow_line(
            &pool, "2026-01-31", "2026-01-31",
            "operating", "cash_collections", "Customer collections", "USD", 999999,
        ).await;
        // Out of range (after)
        insert_cashflow_line(
            &pool, "2026-03-01", "2026-03-01",
            "operating", "cash_collections", "Customer collections", "USD", 888888,
        ).await;

        let from = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let stmt = compute_cashflow(&pool, TENANT, from, to).await.expect("compute");

        let operating = stmt.sections.iter().find(|s| s.activity_type == "operating").unwrap();
        assert_eq!(
            operating.total_by_currency.get("USD").copied().unwrap_or(0),
            100000,
            "only in-range lines should be included"
        );

        cleanup(&pool).await;
    }
}
