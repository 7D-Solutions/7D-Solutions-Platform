/// AR database client for the PLATFORM billing runner.
///
/// Operates exclusively under the PLATFORM app_id — never under a tenant's app_id.
/// Provides:
/// - find_or_create_platform_customer: ensures an AR customer exists for each tenant
/// - create_platform_invoice_idempotent: inserts an invoice only if one does not already exist
///   for the given (tenant, period) correlation key.

use chrono::{NaiveDate, NaiveDateTime};
use sqlx::PgPool;
use uuid::Uuid;

/// The dedicated app_id for all platform-generated billing records.
/// AR invoices and customers under this ID belong to the 7D Platform itself, not any tenant.
pub const PLATFORM_APP_ID: &str = "platform";

/// Build the correlation ID used for idempotency across billing runs.
///
/// Format: `plat-{tenant_id}-{period}` (e.g. `plat-550e8400-...-2026-02`)
pub fn billing_correlation_id(tenant_id: Uuid, period: &str) -> String {
    format!("plat-{}-{}", tenant_id, period)
}

/// Return the plan fee in cents for a given plan_code (v1 hardcoded schedule).
/// bd-pxo5 will introduce a proper pricing table — this is a temporary lookup.
pub fn plan_fee_cents(plan_code: &str) -> i32 {
    match plan_code {
        "monthly" => 2_900,  // $29.00/month
        "annual" => 29_000,  // $290.00/year
        _ => 0,
    }
}

/// Find the existing AR customer for a tenant under the PLATFORM app_id, or create one.
///
/// Uses `external_customer_id = tenant_id` and `email = tenant-{id}@platform.internal`
/// as stable, unique identifiers under the PLATFORM namespace.
pub async fn find_or_create_platform_customer(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<i32, sqlx::Error> {
    let external_id = tenant_id.to_string();
    let email = format!("tenant-{}@platform.internal", tenant_id);

    // Attempt UPSERT — idempotent: if customer already exists, return its id.
    let (id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO ar_customers
            (app_id, external_customer_id, email, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        ON CONFLICT (app_id, external_customer_id) DO UPDATE
            SET updated_at = EXCLUDED.updated_at
        RETURNING id
        "#,
    )
    .bind(PLATFORM_APP_ID)
    .bind(&external_id)
    .bind(&email)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

/// Parse the billing period start date from a "YYYY-MM" string.
fn period_start(period: &str) -> Option<NaiveDateTime> {
    let parts: Vec<&str> = period.splitn(2, '-').collect();
    if parts.len() != 2 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, 1)
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
}

/// Parse the billing period end date (last day of month) from a "YYYY-MM" string.
fn period_end(period: &str) -> Option<NaiveDateTime> {
    let parts: Vec<&str> = period.splitn(2, '-').collect();
    if parts.len() != 2 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    // Last day of the month = first day of next month minus one day
    let next_month = if month == 12 { 1 } else { month + 1 };
    let next_year = if month == 12 { year + 1 } else { year };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.and_hms_opt(23, 59, 59).unwrap())
}

/// Invoice creation result.
pub enum InvoiceResult {
    /// A new invoice was created with the given AR invoice ID.
    Created(i32),
    /// An invoice for this (tenant, period) already exists — no-op.
    AlreadyExists,
}

/// Create an AR invoice for a tenant under the PLATFORM app_id.
///
/// Idempotent: if an invoice with the same correlation_id already exists, returns
/// `InvoiceResult::AlreadyExists` without inserting a duplicate.
pub async fn create_platform_invoice_idempotent(
    pool: &PgPool,
    customer_id: i32,
    tenant_id: Uuid,
    period: &str,
    amount_cents: i32,
) -> Result<InvoiceResult, sqlx::Error> {
    let correlation_id = billing_correlation_id(tenant_id, period);

    // Guard: check if an invoice for this period already exists.
    let existing: Option<(i32,)> = sqlx::query_as(
        "SELECT id FROM ar_invoices WHERE app_id = $1 AND correlation_id = $2",
    )
    .bind(PLATFORM_APP_ID)
    .bind(&correlation_id)
    .fetch_optional(pool)
    .await?;

    if existing.is_some() {
        return Ok(InvoiceResult::AlreadyExists);
    }

    // Stable synthetic tilled_invoice_id (mirrors the format AR uses for external invoices).
    let tilled_invoice_id = format!("plat_{}", correlation_id);
    let billing_start = period_start(period);
    let billing_end = period_end(period);

    let (id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO ar_invoices
            (app_id, tilled_invoice_id, ar_customer_id,
             status, amount_cents, currency,
             correlation_id, billing_period_start, billing_period_end,
             created_at, updated_at)
        VALUES ($1, $2, $3, 'open', $4, 'usd', $5, $6, $7, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(PLATFORM_APP_ID)
    .bind(&tilled_invoice_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(&correlation_id)
    .bind(billing_start)
    .bind(billing_end)
    .fetch_one(pool)
    .await?;

    Ok(InvoiceResult::Created(id))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn billing_correlation_id_is_deterministic() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cid = billing_correlation_id(id, "2026-02");
        assert_eq!(cid, "plat-550e8400-e29b-41d4-a716-446655440000-2026-02");
        assert_eq!(billing_correlation_id(id, "2026-02"), cid);
    }

    #[test]
    fn plan_fee_cents_returns_expected_values() {
        assert_eq!(plan_fee_cents("monthly"), 2_900);
        assert_eq!(plan_fee_cents("annual"), 29_000);
        assert_eq!(plan_fee_cents("unknown"), 0);
        assert_eq!(plan_fee_cents(""), 0);
    }

    #[test]
    fn period_start_parses_correctly() {
        let dt = period_start("2026-02").expect("should parse");
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 2);
        assert_eq!(dt.day(), 1);
    }

    #[test]
    fn period_end_parses_last_day() {
        let dt = period_end("2026-02").expect("should parse");
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 2);
        assert_eq!(dt.day(), 28); // 2026 is not a leap year
    }

    #[test]
    fn period_end_leap_year() {
        let dt = period_end("2024-02").expect("should parse");
        assert_eq!(dt.day(), 29); // 2024 is a leap year
    }

    #[test]
    fn period_end_december_wraps() {
        let dt = period_end("2026-12").expect("should parse");
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 31);
    }

    #[tokio::test]
    async fn platform_billing_full_flow_against_real_db() {
        let ar_db_url = std::env::var("AR_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://ar_user:ar_pass@localhost:5434/ar_db".to_string()
        });
        let ar_pool = match sqlx::PgPool::connect(&ar_db_url).await {
            Ok(p) => p,
            Err(_) => return, // skip if AR DB unavailable
        };

        let tenant_id = Uuid::new_v4();
        let period = "2026-02";

        // Step 1: find_or_create_platform_customer
        let customer_id = find_or_create_platform_customer(&ar_pool, tenant_id)
            .await
            .expect("customer creation should succeed");
        assert!(customer_id > 0);

        // Step 2: idempotent — second call returns same customer_id
        let customer_id2 = find_or_create_platform_customer(&ar_pool, tenant_id)
            .await
            .expect("idempotent call should succeed");
        assert_eq!(customer_id, customer_id2);

        // Step 3: create invoice
        let result = create_platform_invoice_idempotent(
            &ar_pool, customer_id, tenant_id, period, 2_900,
        )
        .await
        .expect("invoice creation should succeed");

        let invoice_id = match result {
            InvoiceResult::Created(id) => id,
            InvoiceResult::AlreadyExists => panic!("should be new invoice"),
        };
        assert!(invoice_id > 0);

        // Step 4: rerun same period → no-op
        let result2 = create_platform_invoice_idempotent(
            &ar_pool, customer_id, tenant_id, period, 2_900,
        )
        .await
        .expect("idempotent call should succeed");
        assert!(matches!(result2, InvoiceResult::AlreadyExists));

        // Cleanup: remove invoice first (FK constraint), then customer
        sqlx::query("DELETE FROM ar_invoices WHERE id = $1")
            .bind(invoice_id)
            .execute(&ar_pool)
            .await
            .ok();
        sqlx::query(
            "DELETE FROM ar_customers WHERE app_id = $1 AND external_customer_id = $2",
        )
        .bind(PLATFORM_APP_ID)
        .bind(tenant_id.to_string())
        .execute(&ar_pool)
        .await
        .ok();
    }
}
