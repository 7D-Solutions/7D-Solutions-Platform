//! AP aging query from the reporting cache.
//!
//! Reads pre-computed aging buckets from `rpt_ap_aging_cache` for a given
//! tenant and as_of date. Returns per-vendor and summary totals.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

// ── Output types ────────────────────────────────────────────────────────────

/// A single vendor's aging buckets for one currency.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct VendorAgingRow {
    pub vendor_id: String,
    pub currency: String,
    pub current_minor: i64,
    pub bucket_1_30_minor: i64,
    pub bucket_31_60_minor: i64,
    pub bucket_61_90_minor: i64,
    pub bucket_over_90_minor: i64,
    pub total_minor: i64,
}

/// Summary totals across all vendors for one currency.
#[derive(Debug, Clone, Serialize)]
pub struct CurrencySummary {
    pub currency: String,
    pub current_minor: i64,
    pub bucket_1_30_minor: i64,
    pub bucket_31_60_minor: i64,
    pub bucket_61_90_minor: i64,
    pub bucket_over_90_minor: i64,
    pub total_minor: i64,
}

/// Complete AP aging report from the reporting cache.
#[derive(Debug, Serialize)]
pub struct ApAgingReport {
    pub as_of: NaiveDate,
    pub vendors: Vec<VendorAgingRow>,
    pub summary_by_currency: Vec<CurrencySummary>,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Query AP aging from the reporting cache for a given tenant and as_of date.
pub async fn query_ap_aging(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<ApAgingReport, anyhow::Error> {
    let vendors: Vec<VendorAgingRow> = sqlx::query_as(
        r#"
        SELECT vendor_id, currency,
               current_minor, bucket_1_30_minor, bucket_31_60_minor,
               bucket_61_90_minor, bucket_over_90_minor, total_minor
        FROM rpt_ap_aging_cache
        WHERE tenant_id = $1 AND as_of = $2
        ORDER BY vendor_id, currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to query AP aging cache: {}", e))?;

    // Aggregate summary by currency
    let mut summary_map: std::collections::BTreeMap<String, CurrencySummary> =
        std::collections::BTreeMap::new();

    for v in &vendors {
        let entry = summary_map
            .entry(v.currency.clone())
            .or_insert_with(|| CurrencySummary {
                currency: v.currency.clone(),
                current_minor: 0,
                bucket_1_30_minor: 0,
                bucket_31_60_minor: 0,
                bucket_61_90_minor: 0,
                bucket_over_90_minor: 0,
                total_minor: 0,
            });
        entry.current_minor += v.current_minor;
        entry.bucket_1_30_minor += v.bucket_1_30_minor;
        entry.bucket_31_60_minor += v.bucket_31_60_minor;
        entry.bucket_61_90_minor += v.bucket_61_90_minor;
        entry.bucket_over_90_minor += v.bucket_over_90_minor;
        entry.total_minor += v.total_minor;
    }

    Ok(ApAgingReport {
        as_of,
        vendors,
        summary_by_currency: summary_map.into_values().collect(),
    })
}

// ── Integrated tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;

    const TENANT: &str = "test-ap-aging-query";

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
        sqlx::query("DELETE FROM rpt_ap_aging_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
    }

    async fn insert_aging(
        pool: &PgPool,
        vendor_id: &str,
        currency: &str,
        as_of: NaiveDate,
        current: i64,
        b30: i64,
        b60: i64,
        b90: i64,
        over90: i64,
    ) {
        let total = current + b30 + b60 + b90 + over90;
        sqlx::query(
            r#"
            INSERT INTO rpt_ap_aging_cache
                (tenant_id, as_of, vendor_id, currency, current_minor,
                 bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
                 bucket_over_90_minor, total_minor, computed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
            "#,
        )
        .bind(TENANT)
        .bind(as_of)
        .bind(vendor_id)
        .bind(currency)
        .bind(current)
        .bind(b30)
        .bind(b60)
        .bind(b90)
        .bind(over90)
        .bind(total)
        .execute(pool)
        .await
        .expect("insert aging");
    }

    #[tokio::test]
    #[serial]
    async fn test_query_returns_vendor_rows() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();
        insert_aging(&pool, "v-001", "USD", today, 10000, 20000, 0, 0, 0).await;
        insert_aging(&pool, "v-002", "USD", today, 5000, 0, 15000, 0, 0).await;

        let report = query_ap_aging(&pool, TENANT, today).await.expect("query");
        assert_eq!(report.vendors.len(), 2);
        assert_eq!(report.summary_by_currency.len(), 1);

        let summary = &report.summary_by_currency[0];
        assert_eq!(summary.currency, "USD");
        assert_eq!(summary.current_minor, 15000);
        assert_eq!(summary.bucket_1_30_minor, 20000);
        assert_eq!(summary.bucket_31_60_minor, 15000);
        assert_eq!(summary.total_minor, 50000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_query_empty_returns_no_rows() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();
        let report = query_ap_aging(&pool, TENANT, today).await.expect("query");
        assert!(report.vendors.is_empty());
        assert!(report.summary_by_currency.is_empty());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_query_multi_currency_summary() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let today = Utc::now().date_naive();
        insert_aging(&pool, "v-001", "USD", today, 10000, 0, 0, 0, 0).await;
        insert_aging(&pool, "v-001", "EUR", today, 5000, 0, 0, 0, 0).await;

        let report = query_ap_aging(&pool, TENANT, today).await.expect("query");
        assert_eq!(report.vendors.len(), 2);
        assert_eq!(report.summary_by_currency.len(), 2);

        let eur = report
            .summary_by_currency
            .iter()
            .find(|s| s.currency == "EUR")
            .expect("EUR");
        assert_eq!(eur.total_minor, 5000);

        let usd = report
            .summary_by_currency
            .iter()
            .find(|s| s.currency == "USD")
            .expect("USD");
        assert_eq!(usd.total_minor, 10000);

        cleanup(&pool).await;
    }
}
