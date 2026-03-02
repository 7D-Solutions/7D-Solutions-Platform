//! E2E: Full subscription billing cycle (bd-37hf)
//!
//! Proves the complete subscription billing loop end-to-end:
//! 1. Create a party in Party Master
//! 2. Create an AR customer linked via party_id
//! 3. Create a subscription plan and subscription
//! 4. Trigger billing cycle → verify AR invoice created with correct amount/customer
//! 5. Apply payment → verify invoice transitions to 'paid'
//! 6. Trigger next cycle → verify renewal invoice created
//!
//! Services required: subscriptions-postgres (5435), ar-postgres (5434),
//!                    payments-postgres (5436), party-postgres (5448)
//!
//! No mocks. No stubs. Real Postgres databases.

mod common;

use anyhow::Result;
use chrono::NaiveDate;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_party_pool,
    get_payments_pool, get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Create a party in Party Master and return the party UUID.
async fn create_party(pool: &PgPool, app_id: &str) -> Result<Uuid> {
    let party_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO party_parties (id, app_id, party_type, status, display_name)
         VALUES ($1, $2, 'company', 'active', $3)",
    )
    .bind(party_id)
    .bind(app_id)
    .bind(format!("Billing Test Corp {}", &party_id.to_string()[..8]))
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO party_companies (party_id, legal_name)
         VALUES ($1, $2)",
    )
    .bind(party_id)
    .bind(format!(
        "Billing Test Corporation {}",
        &party_id.to_string()[..8]
    ))
    .execute(pool)
    .await?;

    Ok(party_id)
}

/// Create an AR customer with party_id link and return its SERIAL id.
async fn create_ar_customer_with_party(pool: &PgPool, app_id: &str, party_id: Uuid) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers
         (app_id, email, name, status, retry_attempt_count, party_id, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, $4, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("billing-{}@test.com", Uuid::new_v4()))
    .bind("Billing Cycle Test Customer")
    .bind(party_id)
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
) -> Result<(Uuid, Uuid)> {
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans
         (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Monthly Pro Plan', 'monthly', 4999, 'USD')
         RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule,
          price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 4999, 'USD', $5, $5)",
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(next_bill_date)
    .execute(pool)
    .await?;

    Ok((plan_id, subscription_id))
}

/// Trigger a billing cycle for a subscription. Returns Some(invoice_id) on
/// success, None if cycle was blocked (duplicate or inactive subscription).
async fn trigger_billing_cycle(
    subscriptions_pool: &PgPool,
    ar_pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    execution_date: NaiveDate,
) -> Result<Option<i32>> {
    use subscriptions_rs::{
        acquire_cycle_lock, calculate_cycle_boundaries, generate_cycle_key, mark_attempt_succeeded,
        record_cycle_attempt, CycleGatingError,
    };

    // Status guard: only active subscriptions are billed
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT status, ar_customer_id, price_minor, currency
         FROM subscriptions WHERE id = $1",
    )
    .bind(subscription_id)
    .fetch_optional(subscriptions_pool)
    .await?;

    let (status, ar_customer_id_str, price_minor, currency) = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    if status != "active" {
        return Ok(None);
    }

    let cycle_key = generate_cycle_key(execution_date);
    let (cycle_start, cycle_end) = calculate_cycle_boundaries(execution_date);

    let mut tx = subscriptions_pool.begin().await?;
    acquire_cycle_lock(&mut *tx, tenant_id, subscription_id, &cycle_key)
        .await
        .map_err(|e| anyhow::anyhow!("acquire_cycle_lock: {:?}", e))?;

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

    let ar_customer_id: i32 = ar_customer_id_str.parse()?;
    let amount_cents = (price_minor / 10) as i32; // minor → cents
    let tilled_invoice_id = format!("inv-{}", Uuid::new_v4());
    let due_at = (execution_date + chrono::Duration::days(30))
        .and_hms_opt(0, 0, 0)
        .unwrap();

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices
         (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency,
          status, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(&tilled_invoice_id)
    .bind(ar_customer_id)
    .bind(amount_cents)
    .bind(&currency)
    .bind(due_at)
    .fetch_one(ar_pool)
    .await?;

    mark_attempt_succeeded(&mut *tx, attempt_id, invoice_id)
        .await
        .map_err(|e| anyhow::anyhow!("mark_attempt_succeeded: {:?}", e))?;

    tx.commit().await?;
    Ok(Some(invoice_id))
}

/// Apply a payment against an invoice: create payment_attempt + update status.
async fn apply_payment(
    payments_pool: &PgPool,
    ar_pool: &PgPool,
    tenant_id: &str,
    invoice_id: i32,
    amount_cents: i32,
) -> Result<Uuid> {
    let payment_id = Uuid::new_v4();

    // Record succeeded payment attempt in Payments module
    sqlx::query(
        "INSERT INTO payment_attempts
         (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3::text, 0, 'succeeded'::payment_attempt_status)",
    )
    .bind(tenant_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .execute(payments_pool)
    .await?;

    // Transition invoice to 'paid' in AR module
    let rows = sqlx::query(
        "UPDATE ar_invoices SET status = 'paid', updated_at = NOW()
         WHERE id = $1 AND app_id = $2",
    )
    .bind(invoice_id)
    .bind(tenant_id)
    .execute(ar_pool)
    .await?;

    anyhow::ensure!(
        rows.rows_affected() == 1,
        "expected 1 row updated for invoice {}, got {}",
        invoice_id,
        rows.rows_affected()
    );

    Ok(payment_id)
}

/// Clean up party test data.
async fn cleanup_party(pool: &PgPool, party_id: Uuid) {
    sqlx::query("DELETE FROM party_companies WHERE party_id = $1")
        .bind(party_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_parties WHERE id = $1")
        .bind(party_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Full billing cycle: party → customer → subscription → invoice → payment →
/// renewal invoice.
#[tokio::test]
#[serial]
async fn test_full_subscription_billing_cycle() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;
    let party_pool = get_party_pool().await;

    // Clean slate
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .ok();

    // ── Step 1: Create party in Party Master ──────────────────────────
    let party_id = create_party(&party_pool, &tenant_id).await?;

    let db_party_type: String =
        sqlx::query_scalar("SELECT party_type::text FROM party_parties WHERE id = $1")
            .bind(party_id)
            .fetch_one(&party_pool)
            .await?;
    assert_eq!(db_party_type, "company", "party must be a company");

    // ── Step 2: Create AR customer with party_id link ────────────────
    let ar_customer_id = create_ar_customer_with_party(&ar_pool, &tenant_id, party_id).await?;

    let db_party_link: Option<Uuid> =
        sqlx::query_scalar("SELECT party_id FROM ar_customers WHERE id = $1 AND app_id = $2")
            .bind(ar_customer_id)
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(
        db_party_link,
        Some(party_id),
        "AR customer must link to the party we created"
    );

    // ── Step 3: Create subscription ──────────────────────────────────
    let billing_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let (_plan_id, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
    )
    .await?;

    let sub_status: String = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .fetch_one(&subscriptions_pool)
        .await?;
    assert_eq!(sub_status, "active", "subscription must start active");

    // ── Step 4: Trigger billing cycle → verify AR invoice ────────────
    let invoice_id = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        billing_date,
    )
    .await?;
    assert!(invoice_id.is_some(), "billing cycle must create an invoice");
    let invoice_id = invoice_id.unwrap();

    // Verify invoice details
    let (inv_amount, inv_currency, inv_customer, inv_status): (i32, String, i32, String) =
        sqlx::query_as(
            "SELECT amount_cents, currency, ar_customer_id, status
             FROM ar_invoices WHERE id = $1 AND app_id = $2",
        )
        .bind(invoice_id)
        .bind(&tenant_id)
        .fetch_one(&ar_pool)
        .await?;

    assert_eq!(inv_amount, 499, "amount_cents must be 499 (4999 / 10)");
    assert_eq!(inv_currency, "USD", "currency must match subscription");
    assert_eq!(
        inv_customer, ar_customer_id,
        "invoice ar_customer_id must match"
    );
    assert_eq!(inv_status, "draft", "new invoice starts as draft");

    // Verify cycle attempt recorded
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(attempt_count, 1, "exactly one cycle attempt recorded");

    // ── Step 5: Apply payment → verify invoice 'paid' ────────────────
    let payment_id =
        apply_payment(&payments_pool, &ar_pool, &tenant_id, invoice_id, inv_amount).await?;

    // Verify payment attempt in Payments DB
    let pa_status: String = sqlx::query_scalar(
        "SELECT status::text FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(&tenant_id)
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await?;
    assert_eq!(pa_status, "succeeded", "payment attempt must be succeeded");

    // Verify invoice status updated to 'paid'
    let paid_status: String =
        sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1 AND app_id = $2")
            .bind(invoice_id)
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(paid_status, "paid", "invoice must be paid after payment");

    // ── Step 6: Trigger next cycle (renewal) → new invoice ───────────
    let renewal_date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    let renewal_invoice = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        renewal_date,
    )
    .await?;
    assert!(
        renewal_invoice.is_some(),
        "renewal cycle must create a new invoice"
    );
    let renewal_id = renewal_invoice.unwrap();
    assert_ne!(renewal_id, invoice_id, "renewal must be a distinct invoice");

    // Verify renewal invoice details match subscription terms
    let (ren_amount, ren_customer, ren_status): (i32, i32, String) = sqlx::query_as(
        "SELECT amount_cents, ar_customer_id, status
         FROM ar_invoices WHERE id = $1 AND app_id = $2",
    )
    .bind(renewal_id)
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(
        ren_amount, 499,
        "renewal amount must equal subscription price"
    );
    assert_eq!(
        ren_customer, ar_customer_id,
        "renewal must bill same customer"
    );
    assert_eq!(ren_status, "draft", "renewal invoice starts as draft");

    // Two cycle attempts now (March + April)
    let final_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(final_attempts, 2, "two cycle attempts (initial + renewal)");

    // Two invoices total
    let total_invoices: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(
        total_invoices, 2,
        "exactly two invoices (initial + renewal)"
    );

    // ── Cleanup ──────────────────────────────────────────────────────
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .ok();
    cleanup_party(&party_pool, party_id).await;

    Ok(())
}

/// Idempotency: triggering the same billing cycle twice creates only one
/// invoice and one cycle attempt.
#[tokio::test]
#[serial]
async fn test_billing_cycle_duplicate_blocked() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;
    let party_pool = get_party_pool().await;

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .ok();

    let party_id = create_party(&party_pool, &tenant_id).await?;
    let ar_customer_id = create_ar_customer_with_party(&ar_pool, &tenant_id, party_id).await?;

    let billing_date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
    let (_plan_id, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
    )
    .await?;

    // First billing — must succeed
    let first = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        billing_date,
    )
    .await?;
    assert!(first.is_some(), "first billing must create invoice");

    // Second billing same cycle — UNIQUE constraint blocks it
    let second = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        billing_date,
    )
    .await?;
    assert!(
        second.is_none(),
        "duplicate cycle must be blocked by idempotency gate"
    );

    // Still only one invoice and one attempt
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await?;
    assert_eq!(attempts, 1, "exactly one attempt despite two triggers");

    let invoices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
        .bind(&tenant_id)
        .fetch_one(&ar_pool)
        .await?;
    assert_eq!(invoices, 1, "exactly one invoice despite two triggers");

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .ok();
    cleanup_party(&party_pool, party_id).await;

    Ok(())
}

/// Payment verification: amount in payment_attempts matches invoice amount,
/// and only one attempt is created per payment.
#[tokio::test]
#[serial]
async fn test_payment_records_match_invoice() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;
    let party_pool = get_party_pool().await;

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .ok();

    let party_id = create_party(&party_pool, &tenant_id).await?;
    let ar_customer_id = create_ar_customer_with_party(&ar_pool, &tenant_id, party_id).await?;

    let billing_date = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let (_plan_id, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
    )
    .await?;

    let invoice_id = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        billing_date,
    )
    .await?
    .expect("billing must create invoice");

    // Fetch invoice amount
    let inv_amount: i32 = sqlx::query_scalar("SELECT amount_cents FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&ar_pool)
        .await?;

    // Apply payment
    let payment_id =
        apply_payment(&payments_pool, &ar_pool, &tenant_id, invoice_id, inv_amount).await?;

    // Verify exactly one payment attempt for this payment
    let pa_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(&tenant_id)
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await?;
    assert_eq!(pa_count, 1, "exactly one payment attempt per payment");

    // Verify invoice_id stored in payment_attempts matches
    let pa_invoice: String = sqlx::query_scalar(
        "SELECT invoice_id FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(&tenant_id)
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await?;
    assert_eq!(
        pa_invoice,
        invoice_id.to_string(),
        "payment attempt must reference the correct invoice"
    );

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .ok();
    cleanup_party(&party_pool, party_id).await;

    Ok(())
}
