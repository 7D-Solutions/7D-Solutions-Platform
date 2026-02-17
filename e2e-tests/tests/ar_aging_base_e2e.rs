//! E2E Test: AR Aging Projection v1 (bd-3cb)
//!
//! Validates the aging projection:
//!   1. Open invoice (not yet due) → appears in `current` bucket
//!   2. Overdue invoice (31-60 days) → appears in `days_31_60` bucket
//!   3. Partial payment → reduces open balance correctly
//!   4. Fully paid invoice → does not appear in aging (balance = 0)
//!   5. Outbox: ar.ar_aging_updated event emitted after refresh

mod common;

use anyhow::Result;
use ar_rs::aging::refresh_aging;
use common::{cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_payments_pool, get_subscriptions_pool, get_gl_pool};
use serial_test::serial;
use sqlx::PgPool;

/// Create a test customer in AR
async fn make_customer(pool: &PgPool, tenant_id: &str) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("aging-{}@test.local", tenant_id))
    .bind(format!("Aging Test {}", tenant_id))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert an open invoice with a specific due_at
async fn make_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    due_offset_days: i64,
) -> Result<i32> {
    // Positive offset = future (not yet due), negative = past (overdue)
    let due_at_expr = if due_offset_days >= 0 {
        format!("NOW() + INTERVAL '{} days'", due_offset_days)
    } else {
        format!("NOW() - INTERVAL '{} days'", due_offset_days.unsigned_abs())
    };

    let invoice_id: i32 = sqlx::query_scalar(&format!(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'usd', {}, NOW(), NOW())
        RETURNING id
        "#,
        due_at_expr
    ))
    .bind(tenant_id)
    .bind(format!("inv_{}", uuid::Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;
    Ok(invoice_id)
}

/// Insert a successful charge against an invoice (simulates payment)
async fn make_payment(
    pool: &PgPool,
    tenant_id: &str,
    invoice_id: i32,
    customer_id: i32,
    amount_cents: i32,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO ar_charges (
            app_id, invoice_id, ar_customer_id, status,
            amount_cents, currency, charge_type, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'succeeded', $4, 'usd', 'one_time', NOW(), NOW())
        "#,
    )
    .bind(tenant_id)
    .bind(invoice_id)
    .bind(customer_id)
    .bind(amount_cents)
    .execute(pool)
    .await?;
    Ok(())
}

/// Count ar.ar_aging_updated outbox events for this tenant
async fn count_aging_outbox_events(pool: &PgPool, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM events_outbox
        WHERE event_type = 'ar.ar_aging_updated'
          AND tenant_id = $1
        "#,
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Test 1: Current (not yet due) invoice appears in correct bucket
#[tokio::test]
#[serial]
async fn test_aging_current_bucket() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    // Invoice due in 10 days (not yet overdue)
    make_invoice(&ar_pool, &tenant_id, customer_id, 10000, 10).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    assert_eq!(snapshot.current_minor, 10000,
        "Invoice due in future should appear in current bucket");
    assert_eq!(snapshot.days_1_30_minor, 0, "No 1-30 overdue");
    assert_eq!(snapshot.days_31_60_minor, 0, "No 31-60 overdue");
    assert_eq!(snapshot.days_61_90_minor, 0, "No 61-90 overdue");
    assert_eq!(snapshot.days_over_90_minor, 0, "No 90+ overdue");
    assert_eq!(snapshot.total_outstanding_minor, 10000);
    assert_eq!(snapshot.invoice_count, 1);

    println!("✅ Current bucket: {} minor units", snapshot.current_minor);

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 2: Overdue invoices appear in correct aging buckets
#[tokio::test]
#[serial]
async fn test_aging_overdue_buckets() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;

    // Invoice 5 days overdue → days_1_30
    make_invoice(&ar_pool, &tenant_id, customer_id, 5000, -5).await?;
    // Invoice 45 days overdue → days_31_60
    make_invoice(&ar_pool, &tenant_id, customer_id, 3000, -45).await?;
    // Invoice 75 days overdue → days_61_90
    make_invoice(&ar_pool, &tenant_id, customer_id, 2000, -75).await?;
    // Invoice 100 days overdue → days_over_90
    make_invoice(&ar_pool, &tenant_id, customer_id, 1000, -100).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    assert_eq!(snapshot.current_minor, 0, "No current invoices");
    assert_eq!(snapshot.days_1_30_minor, 5000, "5 days overdue → 1-30 bucket");
    assert_eq!(snapshot.days_31_60_minor, 3000, "45 days overdue → 31-60 bucket");
    assert_eq!(snapshot.days_61_90_minor, 2000, "75 days overdue → 61-90 bucket");
    assert_eq!(snapshot.days_over_90_minor, 1000, "100 days overdue → 90+ bucket");
    assert_eq!(snapshot.total_outstanding_minor, 11000);
    assert_eq!(snapshot.invoice_count, 4);

    println!("✅ Overdue buckets correct: 1-30={}, 31-60={}, 61-90={}, 90+={}",
        snapshot.days_1_30_minor, snapshot.days_31_60_minor,
        snapshot.days_61_90_minor, snapshot.days_over_90_minor);

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 3: Partial payment reduces open balance in the correct bucket
#[tokio::test]
#[serial]
async fn test_aging_partial_payment() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    // Invoice of 10000, 15 days overdue
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 10000, -15).await?;
    // Partial payment of 3000
    make_payment(&ar_pool, &tenant_id, invoice_id, customer_id, 3000).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Open balance = 10000 - 3000 = 7000, in days_1_30 bucket
    assert_eq!(snapshot.days_1_30_minor, 7000,
        "Open balance (10000 - 3000 partial payment) should be 7000 in 1-30 bucket");
    assert_eq!(snapshot.total_outstanding_minor, 7000);
    assert_eq!(snapshot.invoice_count, 1);

    println!("✅ Partial payment reduces balance: {} minor units remaining", snapshot.days_1_30_minor);

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 4: Fully paid invoice does not appear in aging
#[tokio::test]
#[serial]
async fn test_aging_fully_paid_excluded() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    // Invoice overdue, fully paid
    let invoice_id = make_invoice(&ar_pool, &tenant_id, customer_id, 5000, -20).await?;
    make_payment(&ar_pool, &tenant_id, invoice_id, customer_id, 5000).await?;

    // One unpaid invoice to confirm the updater still runs
    make_invoice(&ar_pool, &tenant_id, customer_id, 2000, 5).await?;

    let snapshot = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Only the unpaid current invoice should appear
    assert_eq!(snapshot.total_outstanding_minor, 2000,
        "Fully paid invoice (5000 - 5000 = 0) must not appear in aging");
    assert_eq!(snapshot.days_1_30_minor, 0,
        "Paid invoice must not show in overdue buckets");
    assert_eq!(snapshot.current_minor, 2000,
        "Unpaid current invoice should appear in current bucket");
    assert_eq!(snapshot.invoice_count, 1,
        "Only 1 open invoice should be counted");

    println!("✅ Fully paid invoice excluded from aging, 2000 minor units outstanding");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 5: ar.ar_aging_updated outbox event emitted on refresh
#[tokio::test]
#[serial]
async fn test_aging_outbox_event_emitted() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    make_invoice(&ar_pool, &tenant_id, customer_id, 8000, 30).await?;

    let pre_count = count_aging_outbox_events(&ar_pool, &tenant_id).await?;
    assert_eq!(pre_count, 0, "No aging events before refresh");

    refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    let post_count = count_aging_outbox_events(&ar_pool, &tenant_id).await?;
    assert_eq!(post_count, 1,
        "One ar.ar_aging_updated event should be enqueued after refresh");

    // Verify envelope metadata
    let (mutation_class, schema_version, source_module): (Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            r#"
            SELECT mutation_class, schema_version, source_module
            FROM events_outbox
            WHERE event_type = 'ar.ar_aging_updated'
              AND tenant_id = $1
            "#,
        )
        .bind(&tenant_id)
        .fetch_one(&ar_pool)
        .await?;

    assert_eq!(mutation_class.as_deref(), Some("DATA_MUTATION"),
        "ar.ar_aging_updated must have mutation_class=DATA_MUTATION");
    assert_eq!(schema_version.as_deref(), Some("1.0.0"),
        "ar.ar_aging_updated must have schema_version=1.0.0");
    assert_eq!(source_module.as_deref(), Some("ar"),
        "ar.ar_aging_updated must have source_module=ar");

    println!("✅ ar.ar_aging_updated outbox event emitted with correct envelope metadata");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// Test 6: Projection is replayable — calling refresh twice produces same result
#[tokio::test]
#[serial]
async fn test_aging_refresh_idempotent() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    let customer_id = make_customer(&ar_pool, &tenant_id).await?;
    make_invoice(&ar_pool, &tenant_id, customer_id, 12000, -60).await?;

    let snapshot1 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;
    let snapshot2 = refresh_aging(&ar_pool, &tenant_id, customer_id).await?;

    // Same ID (upsert should have updated the existing row)
    assert_eq!(snapshot1.id, snapshot2.id,
        "Repeated refresh must upsert the same row, not create duplicates");
    assert_eq!(snapshot1.total_outstanding_minor, snapshot2.total_outstanding_minor,
        "Repeated refresh must produce same totals");
    assert_eq!(snapshot1.days_31_60_minor, snapshot2.days_31_60_minor,
        "Repeated refresh must produce same bucket amounts");

    println!("✅ Projection upsert is idempotent: same row updated, same totals");

    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}
