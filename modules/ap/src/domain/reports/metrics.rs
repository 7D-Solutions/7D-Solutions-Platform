//! Operational metrics snapshot queries for the AP service.
//!
//! These queries are executed on each /metrics scrape to provide
//! current-state gauges for operational observability.
//! No PII is exposed in metric labels — all metrics are cross-tenant totals.

use sqlx::PgPool;

/// Current-state snapshot of AP operational metrics.
pub struct MetricsSnapshot {
    /// Count of bills not yet paid or voided.
    pub open_bills_count: i64,
    /// Open bills past their due_date.
    pub overdue_bills_count: i64,
    /// Sum of total_minor for all open bills (minor currency units).
    pub total_open_amount_minor: i64,
    /// Total payment runs created (all time).
    pub payment_runs_created: i64,
    /// Total allocations created (all time).
    pub allocations_created: i64,
}

/// Fetch current operational metrics snapshot from the database.
///
/// Queries are cross-tenant totals — no per-tenant or per-vendor labels
/// that could expose PII.
pub async fn fetch_snapshot(pool: &PgPool) -> Result<MetricsSnapshot, sqlx::Error> {
    let (open_bills_count, overdue_bills_count, total_open_amount_minor): (i64, i64, i64) =
        sqlx::query_as(
            r#"
            SELECT
                COUNT(*)                                             AS open_bills_count,
                COUNT(*) FILTER (WHERE due_date < NOW())             AS overdue_bills_count,
                COALESCE(SUM(total_minor), 0)::BIGINT                AS total_open_amount_minor
            FROM vendor_bills
            WHERE status NOT IN ('paid', 'voided')
            "#,
        )
        .fetch_one(pool)
        .await?;

    let (payment_runs_created,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM payment_runs")
        .fetch_one(pool)
        .await?;

    let (allocations_created,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ap_allocations")
        .fetch_one(pool)
        .await?;

    Ok(MetricsSnapshot {
        open_bills_count,
        overdue_bills_count,
        total_open_amount_minor,
        payment_runs_created,
        allocations_created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    #[tokio::test]
    async fn test_fetch_snapshot_returns_without_error() {
        let pool = PgPool::connect(&db_url()).await.expect("DB connect failed");

        let snapshot = fetch_snapshot(&pool)
            .await
            .expect("fetch_snapshot should succeed");

        // Gauges must be non-negative
        assert!(snapshot.open_bills_count >= 0);
        assert!(snapshot.overdue_bills_count >= 0);
        assert!(snapshot.total_open_amount_minor >= 0);
        assert!(snapshot.payment_runs_created >= 0);
        assert!(snapshot.allocations_created >= 0);

        // Overdue cannot exceed open
        assert!(snapshot.overdue_bills_count <= snapshot.open_bills_count);
    }
}
