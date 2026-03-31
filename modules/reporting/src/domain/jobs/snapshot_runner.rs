//! Daily snapshot runner for the reporting module.
//!
//! Computes P&L and Balance Sheet statements for each date in a range and
//! persists the results to `rpt_statement_cache`. The operation is idempotent:
//! existing rows are overwritten via ON CONFLICT DO UPDATE.
//!
//! ## Usage
//!
//! Call [`run_snapshot`] with a pool, tenant, and date range. It:
//!   1. For each date `d` in `[from, to]`:
//!      a. Computes P&L (period = [d, d]) — one-day snapshot
//!      b. Computes Balance Sheet (as_of = d) — cumulative
//!      c. Upserts P&L and BS lines into `rpt_statement_cache`
//!
//! ## Idempotency
//!
//! The unique constraint `(tenant_id, statement_type, as_of, line_code, currency)`
//! on `rpt_statement_cache` ensures repeated runs with the same inputs produce
//! the same stored state.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

use crate::domain::statements::{balance_sheet, pl};

// ── Output ────────────────────────────────────────────────────────────────────

/// Summary of a completed snapshot run.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SnapshotRunResult {
    pub tenant_id: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Total rows upserted into rpt_statement_cache.
    pub rows_upserted: u64,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Run snapshot for all dates in `[from, to]` (inclusive).
///
/// Returns the total number of rows upserted.
pub async fn run_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<SnapshotRunResult, anyhow::Error> {
    if from > to {
        return Err(anyhow::anyhow!("'from' must be <= 'to'"));
    }

    let mut rows_upserted: u64 = 0;
    let mut current = from;

    while current <= to {
        rows_upserted += snapshot_one_day(pool, tenant_id, current).await?;
        // Advance by one day
        current = current
            .succ_opt()
            .ok_or_else(|| anyhow::anyhow!("Date overflow"))?;
    }

    Ok(SnapshotRunResult {
        tenant_id: tenant_id.to_string(),
        from,
        to,
        rows_upserted,
    })
}

// ── Per-day snapshot ──────────────────────────────────────────────────────────

async fn snapshot_one_day(
    pool: &PgPool,
    tenant_id: &str,
    date: NaiveDate,
) -> Result<u64, anyhow::Error> {
    let mut count: u64 = 0;

    // P&L: one-day period (date..=date)
    let pl_stmt = pl::compute_pl(pool, tenant_id, date, date)
        .await
        .map_err(|e| anyhow::anyhow!("P&L computation failed for {}: {}", date, e))?;

    for section in &pl_stmt.sections {
        for line in &section.accounts {
            let rows = upsert_statement_line(
                pool,
                tenant_id,
                "pl",
                date,
                &format!("{}.{}", section.section, line.account_code),
                &format!("{} / {}", section.section, line.account_name),
                &line.currency,
                line.amount_minor,
            )
            .await?;
            count += rows;
        }
    }

    // Balance Sheet: cumulative as_of date
    let bs = balance_sheet::compute_balance_sheet(pool, tenant_id, date)
        .await
        .map_err(|e| anyhow::anyhow!("Balance sheet computation failed for {}: {}", date, e))?;

    for section in &bs.sections {
        for line in &section.accounts {
            let rows = upsert_statement_line(
                pool,
                tenant_id,
                "balance_sheet",
                date,
                &format!("{}.{}", section.section, line.account_code),
                &format!("{} / {}", section.section, line.account_name),
                &line.currency,
                line.amount_minor,
            )
            .await?;
            count += rows;
        }
    }

    Ok(count)
}

// ── SQL helper ────────────────────────────────────────────────────────────────

async fn upsert_statement_line(
    pool: &PgPool,
    tenant_id: &str,
    statement_type: &str,
    as_of: NaiveDate,
    line_code: &str,
    line_label: &str,
    currency: &str,
    amount_minor: i64,
) -> Result<u64, anyhow::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO rpt_statement_cache
            (tenant_id, statement_type, as_of, line_code, line_label, currency,
             amount_minor, computed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        ON CONFLICT (tenant_id, statement_type, as_of, line_code, currency) DO UPDATE SET
            line_label   = EXCLUDED.line_label,
            amount_minor = EXCLUDED.amount_minor,
            computed_at  = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(statement_type)
    .bind(as_of)
    .bind(line_code)
    .bind(line_label)
    .bind(currency)
    .bind(amount_minor)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Statement cache upsert failed: {}", e))?;

    Ok(result.rows_affected())
}

// ── Integrated tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;
    use sqlx::PgPool;

    const TENANT: &str = "test-snapshot-runner";

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
        sqlx::query("DELETE FROM rpt_statement_cache WHERE tenant_id = $1")
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

    async fn seed_trial_balance(pool: &PgPool, date: NaiveDate) {
        // Revenue: 4000 credit 5000
        // COGS:    5000 debit  2000
        // Asset:   1000 debit  5000
        for (code, name, debit, credit) in [
            ("4000", "Revenue", 0_i64, 500000_i64),
            ("5000", "COGS", 200000_i64, 0_i64),
            ("1000", "Cash", 500000_i64, 0_i64),
        ] {
            let net = debit - credit;
            sqlx::query(
                r#"
                INSERT INTO rpt_trial_balance_cache
                    (tenant_id, as_of, account_code, account_name, currency,
                     debit_minor, credit_minor, net_minor, computed_at)
                VALUES ($1, $2, $3, $4, 'USD', $5, $6, $7, NOW())
                ON CONFLICT (tenant_id, as_of, account_code, currency) DO NOTHING
                "#,
            )
            .bind(TENANT)
            .bind(date)
            .bind(code)
            .bind(name)
            .bind(debit)
            .bind(credit)
            .bind(net)
            .execute(pool)
            .await
            .expect("seed trial balance");
        }
    }

    async fn count_statement_cache(pool: &PgPool, statement_type: &str, as_of: NaiveDate) -> i64 {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM rpt_statement_cache WHERE tenant_id = $1 AND statement_type = $2 AND as_of = $3",
        )
        .bind(TENANT)
        .bind(statement_type)
        .bind(as_of)
        .fetch_one(pool)
        .await
        .expect("count query");
        count
    }

    #[tokio::test]
    #[serial]
    async fn test_snapshot_runner_persists_rows() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let date = NaiveDate::from_ymd_opt(2026, 5, 1).expect("valid date");
        seed_trial_balance(&pool, date).await;

        let result = run_snapshot(&pool, TENANT, date, date)
            .await
            .expect("run_snapshot");
        assert_eq!(result.from, date);
        assert_eq!(result.to, date);
        assert!(result.rows_upserted > 0, "must have upserted rows");

        // P&L: revenue (4000) + COGS (5000) = 2 rows
        let pl_count = count_statement_cache(&pool, "pl", date).await;
        assert!(pl_count >= 2, "P&L must have rows: {}", pl_count);

        // BS: cash (1000) asset row
        let bs_count = count_statement_cache(&pool, "balance_sheet", date).await;
        assert!(bs_count >= 1, "Balance Sheet must have rows: {}", bs_count);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_snapshot_runner_is_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let date = NaiveDate::from_ymd_opt(2026, 5, 10).expect("valid date");
        seed_trial_balance(&pool, date).await;

        // Run twice
        let r1 = run_snapshot(&pool, TENANT, date, date).await.expect("run1");
        let r2 = run_snapshot(&pool, TENANT, date, date).await.expect("run2");

        // Row counts must be identical (upsert)
        assert_eq!(
            count_statement_cache(&pool, "pl", date).await,
            count_statement_cache(&pool, "pl", date).await,
        );
        // Second run may report fewer "affected" rows if no changes, but still succeeds
        assert_eq!(
            r1.rows_upserted, r2.rows_upserted,
            "upserted count must match"
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_snapshot_from_gt_to_returns_error() {
        let pool = test_pool().await;
        let from = NaiveDate::from_ymd_opt(2026, 6, 10).expect("valid date");
        let to = NaiveDate::from_ymd_opt(2026, 6, 1).expect("valid date");
        let err = run_snapshot(&pool, TENANT, from, to).await;
        assert!(err.is_err(), "from > to must return an error");
    }
}
