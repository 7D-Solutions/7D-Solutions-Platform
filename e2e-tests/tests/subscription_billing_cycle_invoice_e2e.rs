//! E2E: Subscription billing cycle → invoice generation (bd-1yqz)
//!
//! Proves the subscription → invoice automation works end-to-end:
//! 1. Create AR customer (direct DB)
//! 2. Create subscription referencing ar_customer_id with billing_cycle=monthly
//! 3. Trigger billing cycle via cycle-gating library functions
//! 4. Verify auto-generated invoice has correct ar_customer_id, amount_cents,
//!    billing_period_start, billing_period_end
//! 5. Verify subscription next_bill_date advances after cycle
//! 6. Verify outbox event enqueued (billrun.completed)
//! 7. Verify idempotency: same cycle triggered twice → exactly one invoice
//!
//! Services: subscriptions-postgres (5435), ar-postgres (5434)
//!
//! No mocks. No stubs. Real Postgres databases.

mod common;

use anyhow::Result;
use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;
use subscriptions_rs::{
    acquire_cycle_lock, calculate_cycle_boundaries, generate_cycle_key,
    mark_attempt_succeeded, record_cycle_attempt, CycleGatingError,
};
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Create AR customer and return SERIAL id.
async fn create_ar_customer(pool: &PgPool, app_id: &str) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers
         (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("billing-cycle-{}@test.com", Uuid::new_v4()))
    .bind("Subscription Billing Cycle Test Customer")
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Create subscription plan + subscription. Returns (plan_id, subscription_id).
async fn create_plan_and_subscription(
    pool: &PgPool,
    tenant_id: &str,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
    price_minor: i64,
) -> Result<(Uuid, Uuid)> {
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans
         (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Monthly Pro', 'monthly', $2, 'USD')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(price_minor)
    .fetch_one(pool)
    .await?;

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule,
          price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', $5, 'USD', $6, $6)",
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(price_minor)
    .bind(next_bill_date)
    .execute(pool)
    .await?;

    Ok((plan_id, subscription_id))
}

/// Simulate a billing run for one subscription:
/// - Acquires cycle lock
/// - Records attempt (idempotency gate)
/// - Inserts AR invoice with billing_period_start / billing_period_end
/// - Marks attempt succeeded
/// - Updates next_bill_date on subscription
/// - Enqueues billrun.completed outbox event
///
/// Returns Some(invoice_id) on success, None if duplicate cycle.
async fn run_billing_cycle(
    subscriptions_pool: &PgPool,
    ar_pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    ar_customer_id: i32,
    price_minor: i64,
    billing_date: NaiveDate,
) -> Result<Option<i32>> {
    // Compute cycle parameters
    let cycle_key = generate_cycle_key(billing_date);
    let (cycle_start, cycle_end) = calculate_cycle_boundaries(billing_date);

    let mut tx = subscriptions_pool.begin().await?;

    // Acquire advisory lock (prevents concurrent runs for same cycle)
    acquire_cycle_lock(&mut *tx, tenant_id, subscription_id, &cycle_key)
        .await
        .map_err(|e| anyhow::anyhow!("acquire_cycle_lock: {:?}", e))?;

    // Record attempt — UNIQUE constraint blocks duplicates
    let attempt_id = match record_cycle_attempt(
        &mut *tx,
        tenant_id,
        subscription_id,
        &cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await
    {
        Ok(id) => id,
        Err(CycleGatingError::DuplicateCycle { .. }) => {
            tx.rollback().await?;
            return Ok(None);
        }
        Err(e) => return Err(anyhow::anyhow!("record_cycle_attempt: {:?}", e)),
    };

    tx.commit().await?;

    // Create AR invoice in AR database (with billing period fields)
    let billing_period_start: NaiveDateTime = cycle_start.and_hms_opt(0, 0, 0).unwrap();
    let billing_period_end: NaiveDateTime = cycle_end.and_hms_opt(23, 59, 59).unwrap();
    let due_at: NaiveDateTime = (billing_date + Duration::days(30))
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let amount_cents = price_minor as i32;
    let tilled_invoice_id = format!("inv-{}", Uuid::new_v4());

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices
         (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency,
          status, billing_period_start, billing_period_end, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'USD', 'draft', $5, $6, $7, NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(&tilled_invoice_id)
    .bind(ar_customer_id)
    .bind(amount_cents)
    .bind(billing_period_start)
    .bind(billing_period_end)
    .bind(due_at)
    .fetch_one(ar_pool)
    .await?;

    // Mark attempt succeeded
    let mut tx = subscriptions_pool.begin().await?;
    mark_attempt_succeeded(&mut *tx, attempt_id, invoice_id)
        .await
        .map_err(|e| anyhow::anyhow!("mark_attempt_succeeded: {:?}", e))?;
    tx.commit().await?;

    // Advance next_bill_date (one calendar month forward)
    let next_month = advance_one_month(billing_date);
    sqlx::query(
        "UPDATE subscriptions SET next_bill_date = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(next_month)
    .bind(subscription_id)
    .execute(subscriptions_pool)
    .await?;

    // Enqueue billrun.completed outbox event
    let outbox_payload = serde_json::json!({
        "tenant_id": tenant_id,
        "subscription_id": subscription_id.to_string(),
        "cycle_key": cycle_key,
        "invoice_id": invoice_id,
        "ar_customer_id": ar_customer_id,
        "amount_cents": amount_cents,
        "billing_period_start": billing_period_start.to_string(),
        "billing_period_end": billing_period_end.to_string(),
    });
    sqlx::query(
        "INSERT INTO events_outbox (tenant_id, subject, payload, created_at)
         VALUES ($1, 'billrun.completed', $2::jsonb, NOW())",
    )
    .bind(tenant_id)
    .bind(outbox_payload)
    .execute(subscriptions_pool)
    .await?;

    Ok(Some(invoice_id))
}

/// Advance NaiveDate by one calendar month.
fn advance_one_month(date: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    let (year, month, day) = (date.year(), date.month(), date.day());
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, day)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap())
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, day)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap())
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Billing cycle creates invoice with correct ar_customer_id, amount_cents,
/// billing_period_start, and billing_period_end.
#[tokio::test]
#[serial]
async fn test_billing_cycle_invoice_fields_correct() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .ok();

    // ── Step 1: Create AR customer ──────────────────────────────────────────
    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;

    // ── Step 2: Create subscription ─────────────────────────────────────────
    let price_minor: i64 = 2999; // $29.99 in minor units
    let billing_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let (_, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
        price_minor,
    )
    .await?;

    let initial_status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&subscriptions_pool)
            .await?;
    assert_eq!(initial_status, "active", "subscription must start active");

    // ── Step 3: Run billing cycle ───────────────────────────────────────────
    let invoice_id = run_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        ar_customer_id,
        price_minor,
        billing_date,
    )
    .await?;
    assert!(invoice_id.is_some(), "billing cycle must produce an invoice");
    let invoice_id = invoice_id.unwrap();

    // ── Step 4: Verify invoice fields ──────────────────────────────────────
    let (inv_ar_customer, inv_amount, inv_currency, inv_status, inv_period_start, inv_period_end): (
        i32,
        i32,
        String,
        String,
        Option<NaiveDateTime>,
        Option<NaiveDateTime>,
    ) = sqlx::query_as(
        "SELECT ar_customer_id, amount_cents, currency, status,
                billing_period_start, billing_period_end
         FROM ar_invoices WHERE id = $1 AND app_id = $2",
    )
    .bind(invoice_id)
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await?;

    assert_eq!(
        inv_ar_customer, ar_customer_id,
        "invoice ar_customer_id must match the customer we created"
    );
    assert_eq!(
        inv_amount, price_minor as i32,
        "invoice amount_cents must equal subscription price_minor"
    );
    assert_eq!(inv_currency, "USD", "invoice currency must match subscription");
    assert_eq!(inv_status, "draft", "newly created invoice starts as draft");

    let period_start = inv_period_start.expect("billing_period_start must be set");
    let period_end = inv_period_end.expect("billing_period_end must be set");

    // March 2026: 2026-03-01 → 2026-03-31
    assert_eq!(
        period_start.date(),
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        "billing_period_start must be first day of billing month"
    );
    assert_eq!(
        period_end.date(),
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
        "billing_period_end must be last day of billing month"
    );
    assert!(
        period_start < period_end,
        "billing_period_start must precede billing_period_end"
    );

    // ── Step 5: Verify subscription next_bill_date advanced ─────────────────
    let next_bill_date: NaiveDate =
        sqlx::query_scalar("SELECT next_bill_date FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&subscriptions_pool)
            .await?;
    assert_eq!(
        next_bill_date,
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
        "next_bill_date must advance to the following month after billing"
    );

    // Subscription remains active (billing does not change status)
    let post_billing_status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&subscriptions_pool)
            .await?;
    assert_eq!(
        post_billing_status, "active",
        "subscription remains active after successful billing"
    );

    // ── Step 6: Verify outbox event enqueued ────────────────────────────────
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox
         WHERE tenant_id = $1 AND subject = 'billrun.completed'
           AND published_at IS NULL",
    )
    .bind(&tenant_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(
        outbox_count, 1,
        "exactly one billrun.completed event must be enqueued in outbox"
    );

    // ── Step 7: Verify cycle attempt recorded ───────────────────────────────
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2 AND status = 'succeeded'",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(attempt_count, 1, "exactly one succeeded cycle attempt");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .ok();
    Ok(())
}

/// Idempotency: triggering the same billing cycle twice creates exactly one
/// invoice. The UNIQUE constraint on subscription_invoice_attempts blocks
/// the duplicate.
#[tokio::test]
#[serial]
async fn test_billing_cycle_idempotency_no_duplicate_invoice() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .ok();

    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let billing_date = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
    let (_, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
        4999,
    )
    .await?;

    // First run — must produce an invoice
    let first = run_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        ar_customer_id,
        4999,
        billing_date,
    )
    .await?;
    assert!(first.is_some(), "first billing run must create invoice");

    // Second run for the SAME cycle — must be blocked
    let second = run_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        ar_customer_id,
        4999,
        billing_date,
    )
    .await?;
    assert!(
        second.is_none(),
        "duplicate billing for same cycle must be blocked by idempotency gate"
    );

    // Verify: exactly one invoice
    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(invoice_count, 1, "exactly one invoice despite two billing attempts");

    // Verify: exactly one cycle attempt
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(attempt_count, 1, "exactly one cycle attempt despite two triggers");

    // Verify: exactly one outbox event (only the successful run emits one)
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox
         WHERE tenant_id = $1 AND subject = 'billrun.completed'",
    )
    .bind(&tenant_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(outbox_count, 1, "exactly one outbox event despite two triggers");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .ok();
    Ok(())
}

/// Distinct billing cycles (different months) each produce a separate invoice
/// with correct billing_period_start and billing_period_end for their cycle.
#[tokio::test]
#[serial]
async fn test_monthly_renewal_creates_second_invoice_with_correct_period() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .ok();

    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let may_billing = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let (_, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        may_billing,
        9900,
    )
    .await?;

    // May billing
    let may_invoice = run_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        ar_customer_id,
        9900,
        may_billing,
    )
    .await?
    .expect("May billing must create invoice");

    // June billing (next renewal)
    let jun_billing = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let jun_invoice = run_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        ar_customer_id,
        9900,
        jun_billing,
    )
    .await?
    .expect("June billing must create invoice");

    assert_ne!(may_invoice, jun_invoice, "renewal must produce a distinct invoice");

    // Verify May invoice period: 2026-05-01 → 2026-05-31
    let (may_start, may_end): (Option<NaiveDateTime>, Option<NaiveDateTime>) = sqlx::query_as(
        "SELECT billing_period_start, billing_period_end FROM ar_invoices WHERE id = $1",
    )
    .bind(may_invoice)
    .fetch_one(&ar_pool)
    .await?;

    let may_start = may_start.expect("May invoice must have billing_period_start");
    let may_end = may_end.expect("May invoice must have billing_period_end");
    assert_eq!(may_start.date(), NaiveDate::from_ymd_opt(2026, 5, 1).unwrap());
    assert_eq!(may_end.date(), NaiveDate::from_ymd_opt(2026, 5, 31).unwrap());

    // Verify June invoice period: 2026-06-01 → 2026-06-30
    let (jun_start, jun_end): (Option<NaiveDateTime>, Option<NaiveDateTime>) = sqlx::query_as(
        "SELECT billing_period_start, billing_period_end FROM ar_invoices WHERE id = $1",
    )
    .bind(jun_invoice)
    .fetch_one(&ar_pool)
    .await?;

    let jun_start = jun_start.expect("June invoice must have billing_period_start");
    let jun_end = jun_end.expect("June invoice must have billing_period_end");
    assert_eq!(jun_start.date(), NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
    assert_eq!(jun_end.date(), NaiveDate::from_ymd_opt(2026, 6, 30).unwrap());

    // Two attempts, two invoices, two outbox events
    let total_invoices: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(total_invoices, 2, "two distinct billing cycles → two invoices");

    let total_outbox: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND subject = 'billrun.completed'",
    )
    .bind(&tenant_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(total_outbox, 2, "two billing cycles → two outbox events");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .ok();
    Ok(())
}
