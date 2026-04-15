//! AP aging report — read model over vendor_bills and ap_allocations.
//!
//! Computes remaining open balance per bill (total_minor - sum of allocations),
//! then buckets bills by days-past-due relative to `as_of`:
//!
//! - **current**:  due_date >= as_of (not yet overdue)
//! - **1-30**:     as_of - 30 days <= due_date < as_of
//! - **31-60**:    as_of - 60 days <= due_date < as_of - 30 days
//! - **61-90**:    as_of - 90 days <= due_date < as_of - 60 days
//! - **over_90**:  due_date < as_of - 90 days
//!
//! Only includes bills with status `approved` or `partially_paid` that have
//! a positive remaining open balance.
//!
//! Both queries are single-pass: one CTE computes open balances, the outer
//! query groups into buckets — no N+1 behavior.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Output types
// ============================================================================

/// Aging bucket totals for a single currency.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct CurrencyBucket {
    /// ISO 4217 currency code.
    pub currency: String,
    /// Open balance in the current bucket (not yet overdue), in minor units.
    pub current_minor: i64,
    /// Open balance 1–30 days past due, in minor units.
    pub days_1_30_minor: i64,
    /// Open balance 31–60 days past due, in minor units.
    pub days_31_60_minor: i64,
    /// Open balance 61–90 days past due, in minor units.
    pub days_61_90_minor: i64,
    /// Open balance more than 90 days past due, in minor units.
    pub over_90_minor: i64,
    /// Sum of all buckets = total outstanding for this currency.
    pub total_open_minor: i64,
}

/// Aging bucket totals for a single vendor + currency combination.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct VendorBucket {
    pub vendor_id: Uuid,
    pub vendor_name: String,
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub over_90_minor: i64,
    pub total_open_minor: i64,
}

/// Complete aging report response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AgingReport {
    /// The date used as the aging reference point.
    pub as_of: NaiveDate,
    /// Bucket totals grouped by currency.
    pub buckets_by_currency: Vec<CurrencyBucket>,
    /// Per-vendor breakdown — present only when `by_vendor=true` was requested.
    pub vendor_breakdown: Option<Vec<VendorBucket>>,
}

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum AgingError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<AgingError> for platform_http_contracts::ApiError {
    fn from(err: AgingError) -> Self {
        match err {
            AgingError::Database(e) => {
                tracing::error!(error = %e, "Database error in aging report handler");
                Self::internal("An internal error occurred")
            }
        }
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Compute an AP aging report for `tenant_id` as of `as_of`.
///
/// - `as_of`: the cutoff date. Bills due on or after this date are "current";
///   bills due before are bucketed into 1-30, 31-60, 61-90, or over-90 buckets.
/// - `by_vendor`: if true, include a per-vendor breakdown in the response.
pub async fn compute_aging(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
    by_vendor: bool,
) -> Result<AgingReport, AgingError> {
    // Convert to midnight UTC so TIMESTAMPTZ comparisons are deterministic.
    let as_of_dt = as_of
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| {
            AgingError::Database(sqlx::Error::Protocol(
                "invalid date for midnight conversion".into(),
            ))
        })?
        .and_utc();

    let buckets_by_currency =
        super::repo::query_currency_buckets(pool, tenant_id, as_of_dt).await?;

    let vendor_breakdown = if by_vendor {
        Some(super::repo::query_vendor_breakdown(pool, tenant_id, as_of_dt).await?)
    } else {
        None
    };

    Ok(AgingReport {
        as_of,
        buckets_by_currency,
        vendor_breakdown,
    })
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_TENANT: &str = "test-tenant-aging";

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn make_pool() -> PgPool {
        PgPool::connect(&db_url()).await.expect("DB connect")
    }

    // -----------------------------------------------------------------------
    // Setup helpers
    // -----------------------------------------------------------------------

    async fn create_vendor(db: &PgPool) -> Uuid {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendors (vendor_id, tenant_id, name, currency, \
             payment_terms_days, is_active, created_at, updated_at) \
             VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("Aging-Vendor-{}", &vendor_id.to_string()[..8]))
        .execute(db)
        .await
        .expect("create vendor");
        vendor_id
    }

    /// Insert an approved bill with a specific due_date (YYYY-MM-DD string).
    async fn create_approved_bill(
        db: &PgPool,
        vendor_id: Uuid,
        total_minor: i64,
        due_date: &str,
        currency: &str,
    ) -> Uuid {
        let bill_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendor_bills \
             (bill_id, tenant_id, vendor_id, vendor_invoice_ref, \
              currency, total_minor, invoice_date, due_date, status, \
              entered_by, entered_at) \
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), $7::timestamptz, \
                     'approved', 'system', NOW())",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .bind(vendor_id)
        .bind(format!("INV-{}", &bill_id.to_string()[..8]))
        .bind(currency)
        .bind(total_minor)
        .bind(due_date)
        .execute(db)
        .await
        .expect("create bill");
        // Insert a bill line so FK constraints are satisfied
        sqlx::query(
            "INSERT INTO bill_lines \
             (line_id, bill_id, description, quantity, unit_price_minor, \
              line_total_minor, gl_account_code, created_at) \
             VALUES ($1, $2, 'Line', 1.0, $3, $3, '6100', NOW())",
        )
        .bind(Uuid::new_v4())
        .bind(bill_id)
        .bind(total_minor)
        .execute(db)
        .await
        .expect("create bill line");
        bill_id
    }

    /// Apply a partial allocation to a bill.
    async fn allocate(db: &PgPool, bill_id: Uuid, amount_minor: i64, currency: &str) {
        let allocation_id = Uuid::new_v4();
        // Determine allocation_type
        let (total,): (i64,) = sqlx::query_as(
            "SELECT total_minor FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .fetch_one(db)
        .await
        .expect("fetch total");
        let (already,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(amount_minor), 0)::bigint \
             FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .fetch_one(db)
        .await
        .expect("fetch already allocated");
        let alloc_type = if already + amount_minor >= total {
            "full"
        } else {
            "partial"
        };
        let new_status = if already + amount_minor >= total {
            "paid"
        } else {
            "partially_paid"
        };

        sqlx::query(
            "INSERT INTO ap_allocations \
             (allocation_id, bill_id, tenant_id, amount_minor, currency, \
              allocation_type, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, NOW())",
        )
        .bind(allocation_id)
        .bind(bill_id)
        .bind(TEST_TENANT)
        .bind(amount_minor)
        .bind(currency)
        .bind(alloc_type)
        .execute(db)
        .await
        .expect("insert allocation");

        sqlx::query("UPDATE vendor_bills SET status = $1 WHERE bill_id = $2 AND tenant_id = $3")
            .bind(new_status)
            .bind(bill_id)
            .bind(TEST_TENANT)
            .execute(db)
            .await
            .expect("update bill status");
    }

    async fn cleanup(db: &PgPool) {
        for q in [
            "DELETE FROM ap_allocations WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM bill_lines WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM vendor_bills WHERE tenant_id = $1",
            "DELETE FROM vendors WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(db).await.ok();
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn test_empty_returns_no_buckets() {
        let db = make_pool().await;
        cleanup(&db).await;

        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");
        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert!(report.buckets_by_currency.is_empty());
        assert!(report.vendor_breakdown.is_none());

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_current_bucket_bill_not_yet_due() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        // due_date = 2026-02-15, as_of = 2026-01-31 → current
        create_approved_bill(&db, vendor_id, 50000, "2026-02-15", "USD").await;

        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");
        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert_eq!(report.buckets_by_currency.len(), 1);
        let usd = &report.buckets_by_currency[0];
        assert_eq!(usd.currency, "USD");
        assert_eq!(usd.current_minor, 50000);
        assert_eq!(usd.days_1_30_minor, 0);
        assert_eq!(usd.total_open_minor, 50000);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_aging_buckets_correct() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

        // current: due 2026-02-15 (+15 days)
        create_approved_bill(&db, vendor_id, 10000, "2026-02-15", "USD").await;
        // 1-30: due 2026-01-15 (16 days past due)
        create_approved_bill(&db, vendor_id, 20000, "2026-01-15", "USD").await;
        // 31-60: due 2025-12-15 (47 days past due)
        create_approved_bill(&db, vendor_id, 30000, "2025-12-15", "USD").await;
        // 61-90: due 2025-11-15 (77 days past due)
        create_approved_bill(&db, vendor_id, 40000, "2025-11-15", "USD").await;
        // over_90: due 2025-09-30 (123 days past due)
        create_approved_bill(&db, vendor_id, 50000, "2025-09-30", "USD").await;

        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert_eq!(report.buckets_by_currency.len(), 1);
        let usd = &report.buckets_by_currency[0];
        assert_eq!(usd.current_minor, 10000);
        assert_eq!(usd.days_1_30_minor, 20000);
        assert_eq!(usd.days_31_60_minor, 30000);
        assert_eq!(usd.days_61_90_minor, 40000);
        assert_eq!(usd.over_90_minor, 50000);
        assert_eq!(usd.total_open_minor, 150000);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_currency_grouping() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

        create_approved_bill(&db, vendor_id, 10000, "2026-01-15", "USD").await;
        create_approved_bill(&db, vendor_id, 20000, "2026-01-15", "EUR").await;

        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert_eq!(report.buckets_by_currency.len(), 2);
        let eur = &report.buckets_by_currency[0];
        assert_eq!(eur.currency, "EUR");
        assert_eq!(eur.days_1_30_minor, 20000);
        let usd = &report.buckets_by_currency[1];
        assert_eq!(usd.currency, "USD");
        assert_eq!(usd.days_1_30_minor, 10000);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_allocation_reduces_balance() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

        // Bill of 50000, partially paid 20000 → open balance = 30000
        let bill_id = create_approved_bill(&db, vendor_id, 50000, "2026-01-15", "USD").await;
        allocate(&db, bill_id, 20000, "USD").await;

        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert_eq!(report.buckets_by_currency.len(), 1);
        let usd = &report.buckets_by_currency[0];
        // Partial payment: bill is partially_paid, remaining 30000
        assert_eq!(usd.days_1_30_minor, 30000);
        assert_eq!(usd.total_open_minor, 30000);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_fully_paid_bill_excluded() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

        // Bill fully paid → status = 'paid', must not appear in aging
        let bill_id = create_approved_bill(&db, vendor_id, 50000, "2026-01-15", "USD").await;
        allocate(&db, bill_id, 50000, "USD").await;

        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert!(
            report.buckets_by_currency.is_empty(),
            "fully-paid bill must not appear in aging"
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_vendor_breakdown() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

        create_approved_bill(&db, vendor_id, 30000, "2026-01-15", "USD").await;
        create_approved_bill(&db, vendor_id, 20000, "2025-12-15", "USD").await;

        let report = compute_aging(&db, TEST_TENANT, as_of, true)
            .await
            .expect("compute_aging with vendor breakdown");

        let breakdown = report.vendor_breakdown.expect("breakdown present");
        assert_eq!(breakdown.len(), 1); // one vendor, one currency
        let row = &breakdown[0];
        assert_eq!(row.vendor_id, vendor_id);
        assert_eq!(row.days_1_30_minor, 30000);
        assert_eq!(row.days_31_60_minor, 20000);
        assert_eq!(row.total_open_minor, 50000);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_balance_reconciles_with_bill_total_minus_allocations() {
        let db = make_pool().await;
        cleanup(&db).await;

        let vendor_id = create_vendor(&db).await;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

        let bill_id = create_approved_bill(&db, vendor_id, 90000, "2026-01-15", "USD").await;
        // Apply two partial allocations
        allocate(&db, bill_id, 10000, "USD").await;
        allocate(&db, bill_id, 25000, "USD").await;

        // Expected open balance: 90000 - 10000 - 25000 = 55000
        let report = compute_aging(&db, TEST_TENANT, as_of, false)
            .await
            .expect("compute_aging");

        assert_eq!(report.buckets_by_currency.len(), 1);
        let usd = &report.buckets_by_currency[0];
        assert_eq!(usd.total_open_minor, 55000, "balance must reconcile");

        cleanup(&db).await;
    }
}
