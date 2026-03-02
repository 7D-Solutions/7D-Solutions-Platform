//! Unified KPI aggregation from reporting caches.
//!
//! Reads pre-computed data from:
//!   - `rpt_ar_aging_cache`   → AR total outstanding
//!   - `rpt_ap_aging_cache`   → AP total outstanding
//!   - `rpt_cashflow_cache`   → cash collected (operating inflows)
//!   - `rpt_trial_balance_cache` → burn (expense account totals)
//!   - `rpt_kpi_cache`        → inventory value, MRR (if ingested)
//!
//! All values are returned as-of the requested date. Missing cache entries
//! return zero for that KPI — they are not errors.

use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::BTreeMap;

// ── Output types ──────────────────────────────────────────────────────────────

/// Per-currency KPI amounts.
pub type CurrencyMap = BTreeMap<String, i64>;

/// Unified KPI snapshot for a tenant as-of a given date.
#[derive(Debug, Serialize)]
pub struct KpiSnapshot {
    pub as_of: NaiveDate,
    /// Total outstanding AR by currency (all aging buckets summed).
    pub ar_total_outstanding: CurrencyMap,
    /// Total outstanding AP by currency (all aging buckets summed).
    pub ap_total_outstanding: CurrencyMap,
    /// Cash collected YTD by currency (operating inflows from cashflow cache).
    pub cash_collected_ytd: CurrencyMap,
    /// Burn YTD: total expenses from trial balance (expense accounts 5xxx-6xxx).
    pub burn_ytd: CurrencyMap,
    /// Monthly Recurring Revenue, if available from the KPI cache.
    pub mrr: CurrencyMap,
    /// Inventory valuation, if available from the KPI cache.
    pub inventory_value: CurrencyMap,
}

// ── Query functions ───────────────────────────────────────────────────────────

/// Compute the unified KPI snapshot from reporting caches.
pub async fn compute_kpis(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<KpiSnapshot, anyhow::Error> {
    let (ar, ap, cash, burn, mrr, inv) = tokio::try_join!(
        query_ar_totals(pool, tenant_id, as_of),
        query_ap_totals(pool, tenant_id, as_of),
        query_cash_collected_ytd(pool, tenant_id, as_of),
        query_burn_ytd(pool, tenant_id, as_of),
        query_kpi_cache(pool, tenant_id, as_of, "mrr"),
        query_kpi_cache(pool, tenant_id, as_of, "inventory_value"),
    )?;

    Ok(KpiSnapshot {
        as_of,
        ar_total_outstanding: ar,
        ap_total_outstanding: ap,
        cash_collected_ytd: cash,
        burn_ytd: burn,
        mrr,
        inventory_value: inv,
    })
}

/// Sum AR total outstanding per currency from `rpt_ar_aging_cache`.
async fn query_ar_totals(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<CurrencyMap, anyhow::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT currency, COALESCE(SUM(total_minor), 0)::BIGINT
        FROM rpt_ar_aging_cache
        WHERE tenant_id = $1 AND as_of = $2
        GROUP BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("AR totals query failed: {}", e))?;

    Ok(rows.into_iter().collect())
}

/// Sum AP total outstanding per currency from `rpt_ap_aging_cache`.
async fn query_ap_totals(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<CurrencyMap, anyhow::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT currency, COALESCE(SUM(total_minor), 0)::BIGINT
        FROM rpt_ap_aging_cache
        WHERE tenant_id = $1 AND as_of = $2
        GROUP BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("AP totals query failed: {}", e))?;

    Ok(rows.into_iter().collect())
}

/// Sum cash collected YTD from cashflow cache (operating inflows with positive amounts).
///
/// "YTD" = from Jan 1 of as_of's year to as_of.
async fn query_cash_collected_ytd(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<CurrencyMap, anyhow::Error> {
    let year_start = NaiveDate::from_ymd_opt(as_of.year(), 1, 1).unwrap_or(as_of);

    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT currency, COALESCE(SUM(amount_minor), 0)::BIGINT
        FROM rpt_cashflow_cache
        WHERE tenant_id    = $1
          AND activity_type = 'operating'
          AND amount_minor  > 0
          AND period_end   >= $2
          AND period_end   <= $3
        GROUP BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(year_start)
    .bind(as_of)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Cash collected query failed: {}", e))?;

    Ok(rows.into_iter().collect())
}

/// Sum expense account totals (account codes 5000-6999) from trial balance YTD.
///
/// Burn = sum of debit-normal expense account net balances for the year.
async fn query_burn_ytd(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<CurrencyMap, anyhow::Error> {
    let year_start = NaiveDate::from_ymd_opt(as_of.year(), 1, 1).unwrap_or(as_of);

    // Expense accounts: debit-normal, net = debit - credit > 0 = spend
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT currency,
               COALESCE(SUM(GREATEST(0, debit_minor - credit_minor)), 0)::BIGINT
        FROM rpt_trial_balance_cache
        WHERE tenant_id   = $1
          AND account_code >= '5000'
          AND account_code  < '7000'
          AND as_of        >= $2
          AND as_of        <= $3
        GROUP BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(year_start)
    .bind(as_of)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Burn YTD query failed: {}", e))?;

    Ok(rows.into_iter().collect())
}

/// Read a named KPI from `rpt_kpi_cache` for the most recent entry on or before as_of.
async fn query_kpi_cache(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
    kpi_name: &str,
) -> Result<CurrencyMap, anyhow::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT DISTINCT ON (currency) currency, COALESCE(amount_minor, 0)
        FROM rpt_kpi_cache
        WHERE tenant_id = $1
          AND kpi_name  = $2
          AND as_of    <= $3
        ORDER BY currency, as_of DESC
        "#,
    )
    .bind(tenant_id)
    .bind(kpi_name)
    .bind(as_of)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("KPI cache query failed for {}: {}", kpi_name, e))?;

    Ok(rows.into_iter().collect())
}

// ── Integrated tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;
    use sqlx::PgPool;

    const TENANT: &str = "test-kpi-domain";

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
        for table in &[
            "rpt_ar_aging_cache",
            "rpt_ap_aging_cache",
            "rpt_cashflow_cache",
            "rpt_trial_balance_cache",
            "rpt_kpi_cache",
        ] {
            sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
                .bind(TENANT)
                .execute(pool)
                .await
                .ok();
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_kpis_all_zero_when_no_cache_data() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();
        let kpis = compute_kpis(&pool, TENANT, today).await.expect("kpis");

        assert!(kpis.ar_total_outstanding.is_empty());
        assert!(kpis.ap_total_outstanding.is_empty());
        assert!(kpis.cash_collected_ytd.is_empty());
        assert!(kpis.burn_ytd.is_empty());
        assert!(kpis.mrr.is_empty());
        assert!(kpis.inventory_value.is_empty());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_kpis_ar_totals_populated() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();

        sqlx::query(
            r#"INSERT INTO rpt_ar_aging_cache
               (tenant_id, as_of, customer_id, currency, current_minor,
                bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
                bucket_over_90_minor, total_minor, computed_at)
               VALUES ($1, $2, '_total', 'USD', 30000, 10000, 0, 0, 0, 40000, NOW())"#,
        )
        .bind(TENANT)
        .bind(today)
        .execute(&pool)
        .await
        .expect("insert AR");

        let kpis = compute_kpis(&pool, TENANT, today).await.expect("kpis");
        assert_eq!(
            kpis.ar_total_outstanding.get("USD").copied().unwrap_or(0),
            40000
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_kpis_inventory_value_from_cache() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();

        sqlx::query(
            r#"INSERT INTO rpt_kpi_cache
               (tenant_id, as_of, kpi_name, currency, amount_minor, computed_at)
               VALUES ($1, $2, 'inventory_value', 'USD', 150000, NOW())"#,
        )
        .bind(TENANT)
        .bind(today)
        .execute(&pool)
        .await
        .expect("insert KPI");

        let kpis = compute_kpis(&pool, TENANT, today).await.expect("kpis");
        assert_eq!(
            kpis.inventory_value.get("USD").copied().unwrap_or(0),
            150000
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_kpis_ap_totals_populated() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();

        sqlx::query(
            r#"INSERT INTO rpt_ap_aging_cache
               (tenant_id, as_of, vendor_id, currency, current_minor,
                bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
                bucket_over_90_minor, total_minor, computed_at)
               VALUES ($1, $2, 'v-999', 'USD', 20000, 0, 0, 0, 0, 20000, NOW())"#,
        )
        .bind(TENANT)
        .bind(today)
        .execute(&pool)
        .await
        .expect("insert AP");

        let kpis = compute_kpis(&pool, TENANT, today).await.expect("kpis");
        assert_eq!(
            kpis.ap_total_outstanding.get("USD").copied().unwrap_or(0),
            20000
        );

        cleanup(&pool).await;
    }
}
