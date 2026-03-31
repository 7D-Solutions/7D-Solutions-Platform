//! E2E: Subscriptions lifecycle — create, cycle billing, past-due, cancellation (bd-ryz4)
//!
//! ## Coverage
//! 1. Active → billing cycle → invoice created in AR
//! 2. Active → past_due transition with outbox (NATS) event
//! 3. Past_due → suspended (cancellation) with outbox event
//! 4. Suspended subscription excluded from billing (no invoice created)
//! 5. Idempotency: duplicate billing cycle blocked by UNIQUE constraint
//!
//! No mocks — real Postgres databases.

mod common;

use anyhow::Result;
use chrono::NaiveDate;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn create_plan_and_subscription(
    pool: &PgPool,
    tenant_id: &str,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
) -> Result<(Uuid, Uuid)> {
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Monthly Lifecycle Plan', 'monthly', 2999, 'USD') RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency,
          start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 2999, 'USD', $5, $5)",
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

async fn get_status(pool: &PgPool, subscription_id: Uuid) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn count_outbox_events(pool: &PgPool, tenant_id: &str) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn count_cycle_attempts(pool: &PgPool, subscription_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts WHERE subscription_id = $1",
    )
    .bind(subscription_id)
    .fetch_one(pool)
    .await?)
}

/// Simulate a billing cycle for one subscription (mirrors routes.rs bill-run logic).
///
/// Returns `Some(invoice_id)` if invoice was created, `None` if skipped because:
/// - subscription is not active (status guard)
/// - UNIQUE constraint detected a duplicate cycle (idempotency guard)
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

    // Status guard: only active subscriptions are billed (mirrors bill run query)
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT status, ar_customer_id, price_minor, currency FROM subscriptions WHERE id = $1",
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
    let amount_cents = (price_minor / 10) as i32;
    let tilled_invoice_id = format!("inv-{}", Uuid::new_v4());
    let due_at = (execution_date + chrono::Duration::days(30))
        .and_hms_opt(0, 0, 0)
        .unwrap();

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices
         (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency,
          status, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, NOW(), NOW()) RETURNING id",
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

async fn do_cleanup(
    tenant_id: &str,
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    subscriptions_pool: &PgPool,
    gl_pool: &PgPool,
) {
    cleanup_tenant_data(
        ar_pool,
        payments_pool,
        subscriptions_pool,
        gl_pool,
        tenant_id,
    )
    .await
    .expect("cleanup failed");
}

// ============================================================================
// Tests
// ============================================================================

/// Full lifecycle: active → billing → past_due → suspended, NATS events at each step.
#[tokio::test]
#[serial]
async fn test_subscriptions_lifecycle_full() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    do_cleanup(
        &tenant_id,
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
    )
    .await;

    let ar_customer_id = common::create_ar_customer(&ar_pool, &tenant_id).await;
    let billing_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    let (_plan_id, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
    )
    .await?;

    // 1. Initial state: ACTIVE, zero cycle attempts
    assert_eq!(
        get_status(&subscriptions_pool, subscription_id).await?,
        "active"
    );
    assert_eq!(
        count_cycle_attempts(&subscriptions_pool, subscription_id).await?,
        0
    );

    // 2. Billing cycle trigger → invoice must be created in AR
    let invoice_id = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        billing_date,
    )
    .await?;
    assert!(invoice_id.is_some(), "billing cycle must create an invoice");
    assert_eq!(
        count_cycle_attempts(&subscriptions_pool, subscription_id).await?,
        1
    );

    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(invoice_count, 1, "exactly one AR invoice after billing");

    // 3. Transition to PAST_DUE (payment failure) → outbox event emitted
    let outbox_before = count_outbox_events(&subscriptions_pool, &tenant_id).await?;
    subscriptions_rs::transition_to_past_due(
        subscription_id,
        &tenant_id,
        "payment_failed",
        &subscriptions_pool,
    )
    .await
    .map_err(|e| anyhow::anyhow!("transition_to_past_due: {:?}", e))?;

    assert_eq!(
        get_status(&subscriptions_pool, subscription_id).await?,
        "past_due"
    );
    let outbox_after_past_due = count_outbox_events(&subscriptions_pool, &tenant_id).await?;
    assert!(
        outbox_after_past_due > outbox_before,
        "NATS outbox event must be emitted for past_due transition (before={}, after={})",
        outbox_before,
        outbox_after_past_due
    );

    // 4. Transition to SUSPENDED (cancellation) → outbox event emitted
    subscriptions_rs::transition_to_suspended(subscription_id, &tenant_id, "cancelled", &subscriptions_pool)
        .await
        .map_err(|e| anyhow::anyhow!("transition_to_suspended: {:?}", e))?;

    assert_eq!(
        get_status(&subscriptions_pool, subscription_id).await?,
        "suspended"
    );
    let outbox_after_suspended = count_outbox_events(&subscriptions_pool, &tenant_id).await?;
    assert!(
        outbox_after_suspended > outbox_after_past_due,
        "NATS outbox event must be emitted for suspended transition"
    );

    // 5. Re-trigger billing on suspended sub → no invoice (status guard blocks it)
    let next_cycle = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
    let second_invoice = trigger_billing_cycle(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        next_cycle,
    )
    .await?;
    assert!(
        second_invoice.is_none(),
        "suspended subscription must not be billed"
    );

    let final_invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(final_invoice_count, 1, "no new invoice after cancellation");

    do_cleanup(
        &tenant_id,
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
    )
    .await;
    Ok(())
}

/// Idempotency: billing the same cycle twice creates only one invoice.
#[tokio::test]
#[serial]
async fn test_billing_cycle_idempotency() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    do_cleanup(
        &tenant_id,
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
    )
    .await;

    let ar_customer_id = common::create_ar_customer(&ar_pool, &tenant_id).await;
    let billing_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

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

    // Second billing same cycle — UNIQUE constraint must block it
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

    assert_eq!(
        count_cycle_attempts(&subscriptions_pool, subscription_id).await?,
        1
    );

    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(
        invoice_count, 1,
        "exactly one invoice despite two billing attempts"
    );

    do_cleanup(
        &tenant_id,
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
    )
    .await;
    Ok(())
}

/// NATS outbox events emitted for past_due and suspended transitions.
#[tokio::test]
#[serial]
async fn test_status_change_events_emitted() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    do_cleanup(
        &tenant_id,
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
    )
    .await;

    let ar_customer_id = common::create_ar_customer(&ar_pool, &tenant_id).await;
    let billing_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

    let (_plan_id, subscription_id) = create_plan_and_subscription(
        &subscriptions_pool,
        &tenant_id,
        ar_customer_id,
        billing_date,
    )
    .await?;

    assert_eq!(
        count_outbox_events(&subscriptions_pool, &tenant_id).await?,
        0
    );

    // active → past_due: one event
    subscriptions_rs::transition_to_past_due(
        subscription_id,
        &tenant_id,
        "payment_failed",
        &subscriptions_pool,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{:?}", e))?;

    assert_eq!(
        count_outbox_events(&subscriptions_pool, &tenant_id).await?,
        1
    );

    // past_due → suspended: second event
    subscriptions_rs::transition_to_suspended(
        subscription_id,
        &tenant_id,
        "grace_period_expired",
        &subscriptions_pool,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{:?}", e))?;

    assert_eq!(
        count_outbox_events(&subscriptions_pool, &tenant_id).await?,
        2
    );

    // Both events must have the correct type
    let event_types: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM events_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant_id)
    .fetch_all(&subscriptions_pool)
    .await?;

    assert!(
        event_types
            .iter()
            .all(|t| t == "subscriptions.status.changed"),
        "all events must be subscriptions.status.changed, got: {:?}",
        event_types
    );

    do_cleanup(
        &tenant_id,
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
    )
    .await;
    Ok(())
}
