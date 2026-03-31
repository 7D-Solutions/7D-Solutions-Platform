//! AR aging cache queries for the reporting module.
//!
//! Reads pre-computed aging data from `rpt_ar_aging_cache` and returns
//! structured responses for the HTTP layer.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;

/// A single aging bucket row from the reporting cache.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ArAgingRow {
    pub tenant_id: String,
    pub as_of: NaiveDate,
    pub customer_id: String,
    pub currency: String,
    pub current_minor: i64,
    pub bucket_1_30_minor: i64,
    pub bucket_31_60_minor: i64,
    pub bucket_61_90_minor: i64,
    pub bucket_over_90_minor: i64,
    pub total_minor: i64,
}

/// Aggregated aging summary across all customers for a tenant.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ArAgingSummary {
    pub tenant_id: String,
    pub as_of: NaiveDate,
    pub currency: String,
    pub current_minor: i64,
    pub bucket_1_30_minor: i64,
    pub bucket_31_60_minor: i64,
    pub bucket_61_90_minor: i64,
    pub bucket_over_90_minor: i64,
    pub total_minor: i64,
}

/// Fetch all AR aging rows for a tenant on a given date.
pub async fn get_aging_for_tenant(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<Vec<ArAgingRow>, anyhow::Error> {
    let rows = sqlx::query_as::<_, ArAgingRow>(
        r#"
        SELECT tenant_id, as_of, customer_id, currency,
               current_minor, bucket_1_30_minor, bucket_31_60_minor,
               bucket_61_90_minor, bucket_over_90_minor, total_minor
        FROM rpt_ar_aging_cache
        WHERE tenant_id = $1 AND as_of = $2
        ORDER BY total_minor DESC
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Fetch aggregated AR aging summary per currency for a tenant on a given date.
///
/// Sums across all customer_id entries (including `_total` synthetic entries)
/// to produce a per-currency summary.
pub async fn get_aging_summary(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
) -> Result<Vec<ArAgingSummary>, anyhow::Error> {
    let rows: Vec<(String, String, NaiveDate, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT tenant_id, currency, as_of,
               SUM(current_minor)::BIGINT,
               SUM(bucket_1_30_minor)::BIGINT,
               SUM(bucket_31_60_minor)::BIGINT,
               SUM(bucket_61_90_minor)::BIGINT,
               SUM(bucket_over_90_minor)::BIGINT,
               SUM(total_minor)::BIGINT
        FROM rpt_ar_aging_cache
        WHERE tenant_id = $1 AND as_of = $2
        GROUP BY tenant_id, currency, as_of
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(tenant_id, currency, as_of, current, b30, b60, b90, over90, total)| ArAgingSummary {
                tenant_id,
                as_of,
                currency,
                current_minor: current,
                bucket_1_30_minor: b30,
                bucket_31_60_minor: b60,
                bucket_61_90_minor: b90,
                bucket_over_90_minor: over90,
                total_minor: total,
            },
        )
        .collect())
}

// ── Integrated tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TENANT: &str = "test-ar-aging-domain";

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
        sqlx::query("DELETE FROM rpt_ar_aging_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
    }

    async fn seed_aging(
        pool: &PgPool,
        customer_id: &str,
        currency: &str,
        current: i64,
        total: i64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO rpt_ar_aging_cache
                (tenant_id, as_of, customer_id, currency,
                 current_minor, bucket_1_30_minor, bucket_31_60_minor,
                 bucket_61_90_minor, bucket_over_90_minor, total_minor,
                 computed_at)
            VALUES ($1, '2026-02-15', $2, $3, $4, 0, 0, 0, 0, $5, NOW())
            ON CONFLICT (tenant_id, as_of, customer_id, currency) DO UPDATE SET
                current_minor = EXCLUDED.current_minor,
                total_minor = EXCLUDED.total_minor
            "#,
        )
        .bind(TENANT)
        .bind(customer_id)
        .bind(currency)
        .bind(current)
        .bind(total)
        .execute(pool)
        .await
        .expect("seed");
    }

    #[tokio::test]
    #[serial]
    async fn test_get_aging_for_tenant_returns_rows() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        seed_aging(&pool, "_total", "USD", 100000, 150000).await;

        let date = NaiveDate::from_ymd_opt(2026, 2, 15).expect("valid date");
        let rows = get_aging_for_tenant(&pool, TENANT, date)
            .await
            .expect("query");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].current_minor, 100000);
        assert_eq!(rows[0].total_minor, 150000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_aging_summary_aggregates_currencies() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        seed_aging(&pool, "_total", "USD", 100000, 150000).await;
        seed_aging(&pool, "_total", "EUR", 80000, 80000).await;

        let date = NaiveDate::from_ymd_opt(2026, 2, 15).expect("valid date");
        let summary = get_aging_summary(&pool, TENANT, date).await.expect("query");

        assert_eq!(summary.len(), 2);

        let usd = summary.iter().find(|s| s.currency == "USD").expect("USD row");
        assert_eq!(usd.total_minor, 150000);

        let eur = summary.iter().find(|s| s.currency == "EUR").expect("EUR row");
        assert_eq!(eur.total_minor, 80000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_aging_for_tenant_empty_returns_empty() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date");
        let rows = get_aging_for_tenant(&pool, TENANT, date)
            .await
            .expect("query");

        assert!(rows.is_empty());

        cleanup(&pool).await;
    }
}
