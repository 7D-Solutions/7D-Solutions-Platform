//! Aging report repository — SQL queries over vendor_bills and ap_allocations.
//!
//! Both queries are single-pass CTEs (one join computes open balances; the outer
//! query buckets by days-past-due). No N+1 behavior.
//! The service layer (aging.rs) calls these functions and assembles the report.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::aging::{AgingError, CurrencyBucket, VendorBucket};

/// Query currency-level aging buckets for a tenant as of a given date/time.
pub async fn query_currency_buckets(
    pool: &PgPool,
    tenant_id: &str,
    as_of: DateTime<Utc>,
) -> Result<Vec<CurrencyBucket>, AgingError> {
    let rows: Vec<CurrencyBucket> = sqlx::query_as(
        r#"
        WITH bill_open AS (
            SELECT
                b.currency,
                b.due_date,
                (b.total_minor
                    - COALESCE(SUM(a.amount_minor), 0)) AS open_balance_minor
            FROM vendor_bills b
            LEFT JOIN ap_allocations a
                   ON a.bill_id = b.bill_id
                  AND a.tenant_id = b.tenant_id
            WHERE b.tenant_id = $1
              AND b.status IN ('approved', 'partially_paid')
            GROUP BY b.bill_id, b.currency, b.due_date, b.total_minor
            HAVING (b.total_minor - COALESCE(SUM(a.amount_minor), 0)) > 0
        )
        SELECT
            currency,
            COALESCE(SUM(CASE WHEN due_date >= $2
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS current_minor,
            COALESCE(SUM(CASE WHEN due_date >= $2 - INTERVAL '30 days'
                               AND due_date <  $2
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN due_date >= $2 - INTERVAL '60 days'
                               AND due_date <  $2 - INTERVAL '30 days'
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN due_date >= $2 - INTERVAL '90 days'
                               AND due_date <  $2 - INTERVAL '60 days'
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN due_date < $2 - INTERVAL '90 days'
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS over_90_minor,
            COALESCE(SUM(open_balance_minor), 0)::bigint
                AS total_open_minor
        FROM bill_open
        GROUP BY currency
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Query per-vendor aging buckets for a tenant as of a given date/time.
pub async fn query_vendor_breakdown(
    pool: &PgPool,
    tenant_id: &str,
    as_of: DateTime<Utc>,
) -> Result<Vec<VendorBucket>, AgingError> {
    let rows: Vec<VendorBucket> = sqlx::query_as(
        r#"
        WITH bill_open AS (
            SELECT
                b.vendor_id,
                v.name AS vendor_name,
                b.currency,
                b.due_date,
                (b.total_minor
                    - COALESCE(SUM(a.amount_minor), 0)) AS open_balance_minor
            FROM vendor_bills b
            JOIN  vendors v ON v.vendor_id = b.vendor_id
            LEFT JOIN ap_allocations a
                   ON a.bill_id = b.bill_id
                  AND a.tenant_id = b.tenant_id
            WHERE b.tenant_id = $1
              AND b.status IN ('approved', 'partially_paid')
            GROUP BY b.bill_id, b.vendor_id, v.name, b.currency,
                     b.due_date, b.total_minor
            HAVING (b.total_minor - COALESCE(SUM(a.amount_minor), 0)) > 0
        )
        SELECT
            vendor_id,
            vendor_name,
            currency,
            COALESCE(SUM(CASE WHEN due_date >= $2
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS current_minor,
            COALESCE(SUM(CASE WHEN due_date >= $2 - INTERVAL '30 days'
                               AND due_date <  $2
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN due_date >= $2 - INTERVAL '60 days'
                               AND due_date <  $2 - INTERVAL '30 days'
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN due_date >= $2 - INTERVAL '90 days'
                               AND due_date <  $2 - INTERVAL '60 days'
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN due_date < $2 - INTERVAL '90 days'
                              THEN open_balance_minor ELSE 0 END), 0)::bigint
                AS over_90_minor,
            COALESCE(SUM(open_balance_minor), 0)::bigint
                AS total_open_minor
        FROM bill_open
        GROUP BY vendor_id, vendor_name, currency
        ORDER BY vendor_name, currency
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
